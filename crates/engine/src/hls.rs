//! HLS (`.m3u8`) manifest parsing.
//!
//! The browser extension can only hand DM a URL, and for HLS that URL is
//! a **playlist**, not a file: downloading it verbatim gives the user a
//! 300-byte text file (the reported "it only downloaded the playlist"
//! bug). Turning that into media means reading the manifest, choosing a
//! variant, and fetching the segments in order — this module is the
//! reading half (pure, unit-testable); `task.rs` does the fetching.
//!
//! Scope, deliberately:
//!
//! * **Master playlists** are reduced to their highest-bandwidth variant.
//!   A master that puts audio in a *separate* rendition is rejected
//!   (`separate_audio`) — concatenating the video-only variant would
//!   produce a silent file, which is worse than saying so.
//! * **Media playlists** must be VOD (`#EXT-X-ENDLIST`). A live playlist
//!   is an endless stream, not a file, and DM has no notion of "stop
//!   after N minutes".
//! * **Encryption** is reported, not silently ignored: `#EXT-X-KEY`
//!   with `METHOD=AES-128` means the segments are ciphertext, and
//!   writing those bytes to a `.ts` would be a corrupt file.
//! * `#EXT-X-BYTERANGE` and `#EXT-X-MAP` (fMP4) are supported because
//!   they change how bytes are fetched / assembled, not just metadata.
//!
//! Everything else (`#EXT-X-DISCONTINUITY`, `#EXT-X-PROGRAM-DATE-TIME`,
//! `#EXT-X-GAP`, …) is metadata that does not affect the byte stream of
//! a VOD playlist, so it is ignored.

use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HlsError {
    #[error("not an HLS playlist (missing #EXTM3U)")]
    NotPlaylist,
    #[error("could not parse the playlist: {0}")]
    Parse(String),
    #[error("the playlist lists no segments")]
    Empty,
}

/// One `#EXT-X-STREAM-INF` entry of a master playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Absolute URI of the variant's media playlist.
    pub uri: String,
    pub bandwidth: u64,
    /// Height from `RESOLUTION=WxH`, when present.
    pub height: Option<u32>,
}

/// One media segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Absolute URI, resolved against the playlist URL.
    pub uri: String,
    /// `#EXTINF` duration in seconds (0.0 when absent).
    pub duration: f64,
    /// `#EXT-X-BYTERANGE` as `(length, offset)`.
    pub range: Option<(u64, u64)>,
}

/// `#EXT-X-KEY`. `method == "NONE"` means unencrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub method: String,
    pub uri: Option<String>,
    pub iv: Option<String>,
}

/// A media playlist, ready to download.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    pub segments: Vec<Segment>,
    /// `#EXT-X-MAP` init segment (fMP4). Must be written before the
    /// first segment.
    pub init: Option<String>,
    pub key: Option<Key>,
    /// `#EXT-X-ENDLIST` seen: the playlist is complete, not live.
    pub ended: bool,
    /// Sum of the segment durations.
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master {
        variants: Vec<Variant>,
        /// A master that declares an audio rendition with its own URI:
        /// the chosen variant is video-only.
        separate_audio: bool,
    },
    Media(MediaPlaylist),
}

impl MediaPlaylist {
    /// True when the segments are AES-128 ciphertext.
    pub fn encrypted(&self) -> bool {
        self.key
            .as_ref()
            .map(|k| !k.method.eq_ignore_ascii_case("NONE"))
            .unwrap_or(false)
    }
}

/// Is this URL an HLS manifest? `.m3u` is the same idea (a plain list),
/// but it can also be a local radio playlist — only `.m3u8` is claimed.
pub fn is_hls_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".m3u8")
}

/// Split an attribute list (`KEY=VALUE,KEY="a,b"`) on commas that are
/// not inside quotes.
fn attributes(input: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ',' if !quoted => {
                push_attribute(&mut out, &current);
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    push_attribute(&mut out, &current);
    out
}

fn push_attribute(out: &mut Vec<(String, String)>, raw: &str) {
    let entry = raw.trim();
    if entry.is_empty() {
        return;
    }
    if let Some((key, value)) = entry.split_once('=') {
        out.push((
            key.trim().to_ascii_uppercase(),
            value.trim().trim_matches('"').to_string(),
        ));
    }
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn resolve(base: &Url, uri: &str) -> Result<String, HlsError> {
    let uri = uri.trim();
    base.join(uri)
        .map(|u| u.to_string())
        .map_err(|e| HlsError::Parse(format!("bad URI {uri:?}: {e}")))
}

/// Parse a playlist against the URL it was fetched from (relative URIs
/// resolve against it).
pub fn parse(text: &str, base: &Url) -> Result<Playlist, HlsError> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    // Allow a UTF-8 BOM and blank lines before the tag.
    let first = loop {
        match lines.next() {
            Some(line) if line.trim().is_empty() => continue,
            Some(line) => break line.trim_start_matches('\u{feff}').trim().to_string(),
            None => return Err(HlsError::NotPlaylist),
        }
    };
    if !first.starts_with("#EXTM3U") {
        return Err(HlsError::NotPlaylist);
    }

    let mut variants: Vec<Variant> = Vec::new();
    let mut pending_variant: Option<(u64, Option<u32>)> = None;
    let mut separate_audio = false;

    let mut segments: Vec<Segment> = Vec::new();
    let mut pending_duration: Option<f64> = None;
    let mut pending_range: Option<(u64, Option<u64>)> = None;
    let mut last_range_end: u64 = 0;
    let mut init: Option<String> = None;
    let mut key: Option<Key> = None;
    let mut ended = false;

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let attrs = attributes(rest);
            let bandwidth = attr(&attrs, "BANDWIDTH")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let height = attr(&attrs, "RESOLUTION")
                .and_then(|v| v.split_once('x').map(|(_, h)| h.to_string()))
                .and_then(|h| h.parse::<u32>().ok());
            pending_variant = Some((bandwidth, height));
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = attributes(rest);
            let is_audio = attr(&attrs, "TYPE")
                .map(|t| t.eq_ignore_ascii_case("AUDIO"))
                .unwrap_or(false);
            // A rendition with a URI is a separate stream: the variant
            // playlist then carries video only.
            if is_audio && attr(&attrs, "URI").is_some() {
                separate_audio = true;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            // `#EXTINF:<duration>,[<title>]`
            let dur = rest.split(',').next().unwrap_or("").trim();
            pending_duration = Some(dur.parse::<f64>().unwrap_or(0.0));
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let mut parts = rest.splitn(2, '@');
            let length = parts.next().unwrap_or("").trim().parse::<u64>().ok();
            let offset = parts.next().and_then(|o| o.trim().parse::<u64>().ok());
            pending_range = length.map(|l| (l, offset));
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = attributes(rest);
            if let Some(uri) = attr(&attrs, "URI") {
                init = Some(resolve(base, uri)?);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let attrs = attributes(rest);
            let method = attr(&attrs, "METHOD").unwrap_or("NONE").to_string();
            if method.eq_ignore_ascii_case("NONE") {
                key = None;
            } else {
                key = Some(Key {
                    method,
                    uri: match attr(&attrs, "URI") {
                        Some(u) => Some(resolve(base, u)?),
                        None => None,
                    },
                    iv: attr(&attrs, "IV").map(|s| s.to_string()),
                });
            }
            continue;
        }
        if line == "#EXT-X-ENDLIST" {
            ended = true;
            continue;
        }
        if line.starts_with('#') {
            // Other tags (`DISCONTINUITY`, `PROGRAM-DATE-TIME`, …) do
            // not change the bytes we need to concatenate.
            continue;
        }

        // A URI line.
        let uri = resolve(base, line)?;
        if let Some((bandwidth, height)) = pending_variant.take() {
            variants.push(Variant {
                uri,
                bandwidth,
                height,
            });
            continue;
        }
        if let Some(duration) = pending_duration.take() {
            let range = pending_range.take().map(|(length, offset)| {
                let start = offset.unwrap_or(last_range_end);
                last_range_end = start.saturating_add(length);
                (length, start)
            });
            segments.push(Segment {
                uri,
                duration,
                range,
            });
        }
        // A URI with no `#EXTINF` (and no pending variant) is not a
        // segment: skip it rather than invent one.
    }

    if !variants.is_empty() {
        return Ok(Playlist::Master {
            variants,
            separate_audio,
        });
    }
    if segments.is_empty() {
        return Err(HlsError::Empty);
    }
    let duration = segments.iter().map(|s| s.duration).sum();
    Ok(Playlist::Media(MediaPlaylist {
        segments,
        init,
        key,
        ended,
        duration,
    }))
}

/// Highest bandwidth wins; ties keep the first (masters list the
/// "default" variant first in practice).
pub fn best_variant(variants: &[Variant]) -> Option<&Variant> {
    variants
        .iter()
        .max_by_key(|v| (v.bandwidth, v.height.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.test/hls/stream.m3u8").unwrap()
    }

    #[test]
    fn parses_a_media_playlist_with_relative_uris() {
        let text = "#EXTM3U\n\
                    #EXT-X-VERSION:3\n\
                    #EXT-X-TARGETDURATION:6\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:6.000000,\n\
                    segment_000.ts\n\
                    #EXTINF:6.000000,\n\
                    segment_001.ts\n\
                    #EXTINF:3.5,\n\
                    https://other.test/abs.ts\n\
                    #EXT-X-ENDLIST\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert!(media.ended);
        assert_eq!(media.segments.len(), 3);
        assert_eq!(
            media.segments[0].uri,
            "https://cdn.test/hls/segment_000.ts"
        );
        assert_eq!(media.segments[2].uri, "https://other.test/abs.ts");
        assert!((media.duration - 15.5).abs() < 1e-9);
        assert!(!media.encrypted());
    }

    #[test]
    fn a_playlist_without_endlist_is_live() {
        let text = "#EXTM3U\n#EXTINF:6,\na.ts\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert!(!media.ended, "no #EXT-X-ENDLIST means a live stream");
    }

    #[test]
    fn reports_aes_128_instead_of_downloading_ciphertext() {
        let text = "#EXTM3U\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x0123456789ABCDEF0123456789ABCDEF\n\
                    #EXTINF:6,\n\
                    a.ts\n\
                    #EXT-X-ENDLIST\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert!(media.encrypted());
        let key = media.key.unwrap();
        assert_eq!(key.method, "AES-128");
        assert_eq!(key.uri.as_deref(), Some("https://cdn.test/hls/key.bin"));
    }

    #[test]
    fn method_none_clears_the_key() {
        let text = "#EXTM3U\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n\
                    #EXT-X-KEY:METHOD=NONE\n\
                    #EXTINF:6,\n\
                    a.ts\n\
                    #EXT-X-ENDLIST\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert!(!media.encrypted());
    }

    #[test]
    fn resolves_implicit_byte_range_offsets() {
        let text = "#EXTM3U\n\
                    #EXT-X-BYTERANGE:100@0\n\
                    #EXTINF:6,\n\
                    all.ts\n\
                    #EXT-X-BYTERANGE:50\n\
                    #EXTINF:6,\n\
                    all.ts\n\
                    #EXT-X-ENDLIST\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert_eq!(media.segments[0].range, Some((100, 0)));
        assert_eq!(media.segments[1].range, Some((50, 100)));
    }

    #[test]
    fn parses_a_master_and_picks_the_highest_bandwidth() {
        let text = "#EXTM3U\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=1280x720\n\
                    720/index.m3u8\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1920x1080\n\
                    1080/index.m3u8\n";
        let Playlist::Master {
            variants,
            separate_audio,
        } = parse(text, &base()).unwrap()
        else {
            panic!("expected a master playlist");
        };
        assert_eq!(variants.len(), 2);
        assert!(!separate_audio);
        assert_eq!(
            best_variant(&variants).map(|v| v.uri.as_str()),
            Some("https://cdn.test/hls/1080/index.m3u8")
        );
        assert_eq!(variants[1].height, Some(1080));
    }

    #[test]
    fn a_master_with_an_audio_rendition_is_flagged() {
        let text = "#EXTM3U\n\
                    #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"en\",URI=\"audio/en.m3u8\"\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=900000,AUDIO=\"aud\"\n\
                    video.m3u8\n";
        let Playlist::Master {
            separate_audio, ..
        } = parse(text, &base()).unwrap()
        else {
            panic!("expected a master playlist");
        };
        assert!(separate_audio);
    }

    #[test]
    fn carries_the_fmp4_init_segment() {
        let text = "#EXTM3U\n\
                    #EXT-X-MAP:URI=\"init.mp4\"\n\
                    #EXTINF:6,\n\
                    seg1.m4s\n\
                    #EXT-X-ENDLIST\n";
        let Playlist::Media(media) = parse(text, &base()).unwrap() else {
            panic!("expected a media playlist");
        };
        assert_eq!(media.init.as_deref(), Some("https://cdn.test/hls/init.mp4"));
    }

    #[test]
    fn rejects_an_html_error_page() {
        assert_eq!(
            parse("<!doctype html><title>404</title>", &base()).unwrap_err(),
            HlsError::NotPlaylist
        );
    }

    #[test]
    fn rejects_a_playlist_with_no_segments() {
        assert_eq!(
            parse("#EXTM3U\n#EXT-X-ENDLIST\n", &base()).unwrap_err(),
            HlsError::Empty
        );
    }

    #[test]
    fn identifies_hls_urls_but_not_plain_m3u() {
        assert!(is_hls_url("https://cdn.test/a/stream.m3u8?token=1"));
        assert!(is_hls_url("https://cdn.test/a/STREAM.M3U8#x"));
        assert!(!is_hls_url("https://cdn.test/a/radio.m3u"));
        assert!(!is_hls_url("https://cdn.test/a/stream.mp4"));
    }
}
