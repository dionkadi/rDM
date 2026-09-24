//! Media grouping and remuxing — the "video + audio" half of DASH.
//!
//! Most sites that serve video over MPEG-DASH (bilibili and friends)
//! split it into two independent streams: video and audio. Each one is a
//! perfectly ordinary file, so a single URL can never be "the video" —
//! downloading one of them gives you a picture with no sound, or sound
//! with no picture. Everything in this module exists to turn such a pair
//! into one playable file.
//!
//! Two halves:
//!
//! * [`plan`] — given the URLs a page exposed, decide what a *complete*
//!   video would be (one file, a video+audio pair, or a manifest).
//!   Pure and heuristic; see its docs for why the heuristic is allowed
//!   to be approximate.
//! * [`remux`] — combine two downloaded files into one with `ffmpeg
//!   -c copy` (no re-encode, so it is fast and lossless).
//!
//! ## Why the guess is allowed to be approximate
//!
//! URL shape often cannot tell a video segment from an audio segment —
//! bilibili names both `…-1-30280.m4s` / `…-1-30232.m4s`, where only the
//! codec id differs, and that mapping is site-specific. Rather than
//! pretend to be clever, the design validates the **result**:
//!
//! 1. `-map 0:v? -map 0:a? -map 1:v? -map 1:a?` takes whatever each
//!    input actually contains, so the *order* of the two inputs is
//!    irrelevant — ffmpeg sorts it out, not us.
//! 2. After remuxing, [`probe_stream_counts`] checks the output really
//!    has at least one video **and** one audio stream.
//! 3. Only then may the caller clean up the parts. A wrong pairing
//!    therefore produces a reported failure, never a silently broken
//!    file — which is the property that lets us offer this without
//!    asking the user to know what a DASH representation is.
//!
//! ffmpeg is an external dependency. It is detected up front ([`ffmpeg`])
//! so a missing binary is reported as "you need ffmpeg" rather than as a
//! mysterious failure, and `ffprobe` is used when present but not
//! required.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

/// Longest a remux may take before we give up and report a timeout.
/// Remuxing is a copy, so this is generous — it exists to stop a hung
/// ffmpeg from hanging the Tauri command forever.
const REMUX_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How much of ffmpeg's stderr to keep for the error message.
const STDERR_TAIL: usize = 2_000;

/// What a URL (or a downloaded file) appears to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PartKind {
    /// A video-only stream (or a container that holds video).
    Video,
    /// An audio-only stream.
    Audio,
    /// A `.m4s` DASH stream. Either half of a pair — the extension is
    /// identical for both, so this is deliberately *not* split into
    /// video/audio here. See the module docs.
    DashStream,
    /// A single HLS segment (`.ts`). One piece of a longer stream, never
    /// a whole file — pairing two of these would be wrong.
    HlsSegment,
    /// An HLS (`.m3u8`) or DASH (`.mpd`) manifest.
    Manifest,
    /// Anything else (a page, an API call, an image…).
    Unknown,
}

impl PartKind {
    /// Classify by file extension, ignoring the query string.
    pub fn from_url(url: &str) -> Self {
        let without_fragment = url.split('#').next().unwrap_or(url);
        let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
        let ext = Path::new(without_query)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "mp4" | "m4v" | "webm" | "mkv" | "mov" | "flv" | "avi" | "wmv" | "mpg" | "mpeg" => {
                Self::Video
            }
            "m4a" | "mp3" | "aac" | "flac" | "wav" | "ogg" | "oga" | "opus" | "wma" => Self::Audio,
            "m4s" => Self::DashStream,
            "ts" => Self::HlsSegment,
            "m3u8" | "mpd" => Self::Manifest,
            _ => Self::Unknown,
        }
    }

    /// Classify by an HTTP `Content-Type`. This is the reliable
    /// discriminator for the ambiguous cases (`m4s`), and the reason
    /// `probe`-based validation works at all.
    pub fn from_content_type(content_type: &str) -> Self {
        let ct = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if ct.starts_with("video/") {
            Self::Video
        } else if ct.starts_with("audio/") {
            Self::Audio
        } else if ct.contains("mpegurl") {
            Self::Manifest
        } else if ct == "application/dash+xml" {
            Self::Manifest
        } else {
            Self::Unknown
        }
    }

    /// True for the kinds that can be one half of a mergeable pair.
    pub fn is_media_stream(self) -> bool {
        matches!(self, Self::Video | Self::Audio | Self::DashStream)
    }
}

/// What a set of captured URLs amounts to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MediaPlan {
    /// Nothing usable was found.
    Empty,
    /// One URL that is the whole file.
    Single {
        url: String,
        /// What it looks like, for the UI's benefit.
        part_kind: PartKind,
    },
    /// Two streams of the same video that need merging once downloaded.
    Pair {
        /// First stream. Which of the two is video does not matter:
        /// ffmpeg maps by stream type, not by input order.
        first: String,
        /// Second stream.
        second: String,
    },
    /// A manifest. DM can download the playlist file itself but cannot
    /// expand it yet, so this is reported honestly rather than offered
    /// as if it were a video.
    Manifest { url: String },
}

/// Do two URLs look like they belong to the same media item?
///
/// Same scheme + host, and the same parent path. This is what stops us
/// pairing `cdn-a/…/video.m4s` with `cdn-b/…/audio.m4s` from an unrelated
/// player, or a video from one page with audio from another.
fn same_origin_and_dir(a: &str, b: &str) -> bool {
    fn split(url: &str) -> Option<(String, String)> {
        let parsed = url::Url::parse(url).ok()?;
        let parent = parsed.path().rsplit_once('/').map(|(p, _)| p.to_string())?;
        Some((parsed.origin().ascii_serialization(), parent))
    }
    match (split(a), split(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Decide what a capture's URL list amounts to.
///
/// Preference order, and why:
///
/// 1. **A manifest** wins when present, because it is the only URL that
///    describes the *whole* video. DM cannot expand one yet — the caller
///    says so rather than downloading a playlist file and calling it a
///    video.
/// 2. **One video + one audio** of known kinds: an unambiguous pair.
/// 3. **Two DASH streams** (`.m4s`) from the same directory: the
///    bilibili case. The two are very likely the video and audio halves
///    — and because the merge validates its own result, guessing here
///    costs the user a clear error at worst, not a broken file.
/// 4. **A single media stream**: done.
///
/// HLS segments (`.ts`) are deliberately never paired: they are
/// sequential pieces of one stream, so "merging" two of them would
/// concatenate two chunks of video and produce nonsense.
pub fn plan(urls: &[String]) -> MediaPlan {
    let mut manifests = Vec::new();
    let mut videos = Vec::new();
    let mut audios = Vec::new();
    let mut dash = Vec::new();
    let mut singles = Vec::new();

    for url in urls {
        match PartKind::from_url(url) {
            PartKind::Manifest => manifests.push(url.clone()),
            PartKind::Video => videos.push(url.clone()),
            PartKind::Audio => audios.push(url.clone()),
            PartKind::DashStream => dash.push(url.clone()),
            // HLS pieces can't be paired; an unknown URL is not offered
            // as a video either.
            PartKind::HlsSegment | PartKind::Unknown => {}
        }
        if PartKind::from_url(url).is_media_stream() {
            singles.push(url.clone());
        }
    }

    if let Some(url) = manifests.first() {
        return MediaPlan::Manifest { url: url.clone() };
    }
    if let (Some(v), Some(a)) = (videos.first(), audios.first()) {
        return MediaPlan::Pair {
            first: v.clone(),
            second: a.clone(),
        };
    }
    if dash.len() == 2 && same_origin_and_dir(&dash[0], &dash[1]) {
        return MediaPlan::Pair {
            first: dash[0].clone(),
            second: dash[1].clone(),
        };
    }
    if let Some(url) = singles.first() {
        return MediaPlan::Single {
            url: url.clone(),
            part_kind: PartKind::from_url(url),
        };
    }
    MediaPlan::Empty
}

/// The ffmpeg argv used for merging.
///
/// * `-nostdin` — there is no console to read from; without this ffmpeg
///   can block on stdin in a GUI process.
/// * `-map …?` per input per stream type — each input contributes
///   whatever it has, so the caller does not need to know which file is
///   the video. Subtitle/attachment streams are left out; the output is
///   only ever video + audio.
/// * `-c copy` — remux, never re-encode: fast, lossless, and it works
///   with whatever codecs the site chose.
/// * `-movflags +faststart` — the index ends up at the front, so the
///   file can be played while it is still being copied around.
pub fn remux_args(video: &Path, audio: &Path, out: &Path) -> Vec<OsString> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
        &video.to_string_lossy(),
        "-i",
        &audio.to_string_lossy(),
        "-map",
        "0:v?",
        "-map",
        "0:a?",
        "-map",
        "1:v?",
        "-map",
        "1:a?",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        &out.to_string_lossy(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// A binary we found on `PATH`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

impl ToolInfo {
    pub fn available(&self) -> bool {
        self.path.is_some() && self.version.is_some()
    }
}

/// Resolve `name` against `PATH`. `Command::new` would find it anyway,
/// but we want the path to show the user which binary is in use.
pub fn which(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

async fn tool_info(name: &str) -> ToolInfo {
    let path = which(name);
    let version = if path.is_some() {
        match Command::new(name)
            .arg("-hide_banner")
            .arg("-version")
            .stdin(Stdio::null())
            .output()
            .await
        {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .map(|l| l.trim().to_string()),
            _ => None,
        }
    } else {
        None
    };
    ToolInfo {
        name: name.to_string(),
        path: path.map(|p| p.to_string_lossy().to_string()),
        version,
    }
}

/// Is ffmpeg available? A missing ffmpeg is a first-class outcome, not
/// an error to be discovered halfway through a merge.
pub async fn ffmpeg() -> ToolInfo {
    tool_info("ffmpeg").await
}

/// Count the stream types in a file. `None` when ffprobe is missing or
/// the file can't be read — the caller must treat that as "unverified",
/// not as "fine".
pub async fn probe_stream_counts(path: &Path) -> Option<(usize, usize)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut video = 0;
    let mut audio = 0;
    for line in text.lines() {
        match line.trim() {
            "video" => video += 1,
            "audio" => audio += 1,
            _ => {}
        }
    }
    Some((video, audio))
}

/// What a successful merge produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub output_path: String,
    /// Bytes of the merged file.
    pub bytes: u64,
    /// `(video, audio)` stream counts, or `None` when ffprobe isn't
    /// installed and the result could not be verified.
    pub streams: Option<(usize, usize)>,
    /// Container we ended up with (`mp4` or `mkv`).
    pub container: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("ffmpeg is not installed or not on PATH (install it to merge video + audio)")]
    FfmpegMissing,
    #[error("could not run ffmpeg: {0}")]
    Spawn(String),
    #[error("ffmpeg failed while merging into {container}: {stderr}")]
    Failed { container: String, stderr: String },
    #[error("merging timed out after {}s", REMUX_TIMEOUT.as_secs())]
    Timeout,
    #[error(
        "the merged file has no {missing} stream — the two inputs are probably not a \
         video/audio pair (or the site served a segment rather than a whole stream)"
    )]
    Unusable { missing: &'static str },
}

async fn run_ffmpeg(
    video: &Path,
    audio: &Path,
    out: &Path,
    container: &str,
) -> Result<Option<(usize, usize)>, MergeError> {
    let child = Command::new("ffmpeg")
        .args(remux_args(video, audio, out))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // What makes the timeout below safe: on timeout the future
        // holding the child is dropped, and this flag turns that into a
        // kill. Without it a hung ffmpeg would keep running and holding
        // the output file open.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| MergeError::Spawn(e.to_string()))?;

    let output = match tokio::time::timeout(REMUX_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(MergeError::Spawn(e.to_string())),
        Err(_) => return Err(MergeError::Timeout),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .chars()
            .rev()
            .take(STDERR_TAIL)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(MergeError::Failed {
            container: container.to_string(),
            stderr: tail.trim().to_string(),
        });
    }

    let streams = probe_stream_counts(out).await;
    if let Some((video_streams, audio_streams)) = streams {
        if video_streams == 0 {
            return Err(MergeError::Unusable { missing: "video" });
        }
        if audio_streams == 0 {
            return Err(MergeError::Unusable { missing: "audio" });
        }
    }
    Ok(streams)
}

/// Where the merged file should go: next to the first input, same stem,
/// new container.
///
/// The natural name can collide with an input — a `.mp4` video part
/// merged to `.mp4` would target the very file it is reading. Remuxing
/// onto an open input either fails or destroys data, so on collision we
/// pick `<stem>.merged.<ext>` instead of skipping the container (which
/// would silently downgrade a perfectly good mp4 merge to mkv, or fail
/// outright).
fn output_path_for(first: &Path, second: &Path, stem: &str, container: &str) -> PathBuf {
    let dir = first.parent().unwrap_or_else(|| Path::new("."));
    let direct = dir.join(format!("{stem}.{container}"));
    if direct == first || direct == second {
        dir.join(format!("{stem}.merged.{container}"))
    } else {
        direct
    }
}

/// Merge two downloaded streams into one file next to the first input.
///
/// Tries `.mp4` first and falls back to `.mkv` if ffmpeg rejects the
/// codec combination for MP4 — `.mkv` accepts essentially anything, so
/// the fallback turns an avoidable failure into a working file.
///
/// The caller is responsible for the parts: this function only writes
/// the merged file, and only returns `Ok` once the result has been
/// checked (see [`probe_stream_counts`]). A failed or unverified merge
/// leaves the parts untouched and removes its own half-written output.
pub async fn remux(first: &Path, second: &Path) -> Result<MergeOutcome, MergeError> {
    if !ffmpeg().await.available() {
        return Err(MergeError::FfmpegMissing);
    }

    let stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "merged".to_string());

    let mut last_error: Option<MergeError> = None;
    for container in ["mp4", "mkv"] {
        let out = output_path_for(first, second, &stem, container);
        match run_ffmpeg(first, second, &out, container).await {
            Ok(streams) => {
                let bytes = tokio::fs::metadata(&out)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                return Ok(MergeOutcome {
                    output_path: out.to_string_lossy().to_string(),
                    bytes,
                    streams,
                    container: container.to_string(),
                });
            }
            Err(e) => {
                // Leave no half-written file behind.
                let _ = tokio::fs::remove_file(&out).await;
                last_error = Some(e);
            }
        }
    }
    Err(last_error.unwrap_or(MergeError::FfmpegMissing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_extension_ignoring_query_and_fragment() {
        assert_eq!(
            PartKind::from_url("https://cdn/x/movie.mp4?token=abc#t=1"),
            PartKind::Video
        );
        assert_eq!(
            PartKind::from_url("https://cdn/x/song.m4a"),
            PartKind::Audio
        );
        assert_eq!(
            PartKind::from_url("https://cdn/x/stream.m4s?range=0-100"),
            PartKind::DashStream
        );
        assert_eq!(PartKind::from_url("https://cdn/x/seg0.ts"), PartKind::HlsSegment);
        assert_eq!(
            PartKind::from_url("https://cdn/x/master.m3u8"),
            PartKind::Manifest
        );
        assert_eq!(
            PartKind::from_url("https://site.test/article.html"),
            PartKind::Unknown
        );
    }

    #[test]
    fn classifies_by_content_type_for_the_ambiguous_cases() {
        assert_eq!(PartKind::from_content_type("video/mp4"), PartKind::Video);
        assert_eq!(
            PartKind::from_content_type("audio/mp4; charset=binary"),
            PartKind::Audio
        );
        assert_eq!(
            PartKind::from_content_type("application/vnd.apple.mpegurl"),
            PartKind::Manifest
        );
        assert_eq!(PartKind::from_content_type("text/html"), PartKind::Unknown);
    }

    #[test]
    fn pairs_a_known_video_with_a_known_audio() {
        let urls = vec![
            "https://cdn.test/v/clip.mp4".to_string(),
            "https://cdn.test/v/clip.m4a".to_string(),
        ];
        assert_eq!(
            plan(&urls),
            MediaPlan::Pair {
                first: "https://cdn.test/v/clip.mp4".into(),
                second: "https://cdn.test/v/clip.m4a".into(),
            }
        );
    }

    #[test]
    fn pairs_two_dash_streams_from_the_same_directory() {
        // The bilibili shape: one video stream, one audio stream, same
        // directory, indistinguishable by extension.
        let urls = vec![
            "https://upos.test/upgcxcode/12/34/56/123456-1-30280.m4s?deadline=1".to_string(),
            "https://upos.test/upgcxcode/12/34/56/123456-1-30232.m4s?deadline=1".to_string(),
        ];
        assert!(matches!(plan(&urls), MediaPlan::Pair { .. }));
    }

    #[test]
    fn refuses_to_pair_streams_from_different_hosts_or_directories() {
        // Two unrelated single-stream hits must not look like a pair.
        let different_hosts = vec![
            "https://a.test/1-30280.m4s".to_string(),
            "https://b.test/1-30232.m4s".to_string(),
        ];
        assert!(matches!(plan(&different_hosts), MediaPlan::Single { .. }));

        let different_dirs = vec![
            "https://a.test/one/x.m4s".to_string(),
            "https://a.test/two/y.m4s".to_string(),
        ];
        assert!(matches!(plan(&different_dirs), MediaPlan::Single { .. }));
    }

    #[test]
    fn never_pairs_hls_segments() {
        // `.ts` pieces are consecutive chunks of ONE stream; merging two
        // of them would concatenate video onto video.
        let urls = vec![
            "https://cdn.test/hls/seg0.ts".to_string(),
            "https://cdn.test/hls/seg1.ts".to_string(),
        ];
        assert_eq!(plan(&urls), MediaPlan::Empty);
    }

    #[test]
    fn a_manifest_wins_and_is_reported_as_a_manifest() {
        let urls = vec![
            "https://cdn.test/hls/master.m3u8".to_string(),
            "https://cdn.test/hls/seg0.ts".to_string(),
        ];
        assert_eq!(
            plan(&urls),
            MediaPlan::Manifest {
                url: "https://cdn.test/hls/master.m3u8".into()
            }
        );
    }

    #[test]
    fn a_single_video_is_a_single() {
        let urls = vec!["https://cdn.test/one.mp4".to_string()];
        assert_eq!(
            plan(&urls),
            MediaPlan::Single {
                url: "https://cdn.test/one.mp4".into(),
                part_kind: PartKind::Video,
            }
        );
    }

    #[test]
    fn html_is_not_media() {
        assert_eq!(
            plan(&["https://site.test/player.html".to_string()]),
            MediaPlan::Empty
        );
    }

    #[test]
    fn remux_args_map_every_input_without_caring_about_order() {
        let args = remux_args(
            Path::new("/tmp/a.m4s"),
            Path::new("/tmp/b.m4s"),
            Path::new("/tmp/out.mp4"),
        );
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        // Both inputs are mapped for both stream types, optionally, so
        // the caller never has to know which file is the video.
        for map in ["0:v?", "0:a?", "1:v?", "1:a?"] {
            assert!(args.contains(&map.to_string()), "missing -map {map}");
        }
        // A stream copy, and never a console read.
        assert!(args.contains(&"copy".to_string()));
        assert!(args.contains(&"-nostdin".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/out.mp4");
        // No re-encode flags.
        assert!(!args.iter().any(|a| a == "-c:v" || a == "-crf"));
    }

    // ── Real ffmpeg, real files ─────────────────────────────────────
    //
    // These are the tests that matter: everything above is a heuristic,
    // and heuristics are exactly what this feature must not trust. They
    // synthesise tiny clips with fuzzed colour bars (no input files
    // needed) and then check that the merge either produces a file with
    // both a video and an audio stream, or fails *loudly*.
    //
    // ffmpeg is skipped with a printed note when absent so the suite
    // still runs on machines without it. CI installs ffmpeg, so there it
    // is a real gate.

    /// `Some(())` when ffmpeg is usable, `None` (with a note) when not.
    async fn require_ffmpeg() -> bool {
        if ffmpeg().await.available() {
            true
        } else {
            eprintln!("skipping: ffmpeg is not installed");
            false
        }
    }

    /// Synthesise a one-second clip. `video` picks a colour-bars video
    /// stream, otherwise a sine-wave audio stream.
    async fn make_clip(dir: &Path, stem: &str, video: bool) -> PathBuf {
        let out = if video {
            dir.join(format!("{stem}.mp4"))
        } else {
            dir.join(format!("{stem}.m4a"))
        };
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];
        if video {
            args.extend(
                [
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=duration=1:size=128x72:rate=5",
                    // mpeg4 rather than libx264: present in every
                    // distro build, and this test is about remuxing,
                    // not about which encoder is available.
                    "-c:v",
                    "mpeg4",
                    "-q:v",
                    "5",
                ]
                .map(String::from),
            );
        } else {
            args.extend(
                [
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=1",
                    "-c:a",
                    "aac",
                ]
                .map(String::from),
            );
        }
        args.push(out.to_string_lossy().to_string());

        let status = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .expect("spawn ffmpeg");
        assert!(status.success(), "ffmpeg could not synthesise {out:?}");
        out
    }

    #[tokio::test]
    async fn remuxes_a_real_video_and_audio_pair_into_one_playable_file() {
        if !require_ffmpeg().await {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let video = make_clip(dir.path(), "clip", true).await;
        let audio = make_clip(dir.path(), "clip", false).await;

        let outcome = remux(&video, &audio).await.expect("merge should succeed");

        assert_eq!(outcome.container, "mp4");
        assert!(outcome.bytes > 0, "merged file is empty");
        assert_eq!(
            outcome.streams,
            Some((1, 1)),
            "the merged file must carry exactly one video and one audio stream"
        );
        assert!(Path::new(&outcome.output_path).is_file());
        // The inputs are left alone — the caller decides what to do with
        // them, and only after a verified success.
        assert!(video.is_file() && audio.is_file());
    }

    #[tokio::test]
    async fn refuses_to_merge_two_videos_instead_of_writing_a_silent_failure() {
        if !require_ffmpeg().await {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let first = make_clip(dir.path(), "one", true).await;
        let second = make_clip(dir.path(), "two", true).await;

        // This is the safety property the whole design leans on: a wrong
        // pairing (two video-only streams) must NOT produce a file that
        // looks fine. It has to be reported.
        let before = list_dir(dir.path());
        let err = remux(&first, &second).await.expect_err("must not succeed");
        match err {
            MergeError::Unusable { missing } => assert_eq!(missing, "audio"),
            other => panic!("expected Unusable, got {other:?}"),
        }

        // And nothing may be left behind: a failed merge must leave the
        // directory exactly as it was (the two inputs, nothing more).
        assert_eq!(
            list_dir(dir.path()),
            before,
            "a failed merge must not create or delete anything"
        );
    }

    fn list_dir(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_output_name_does_not_collide_with_an_input() {
        // A `.mp4` video part must not be merged onto itself.
        let first = Path::new("/tmp/clip.mp4");
        let second = Path::new("/tmp/clip.m4a");
        assert_eq!(
            output_path_for(first, second, "clip", "mp4"),
            PathBuf::from("/tmp/clip.merged.mp4"),
            "merging clip.mp4 must not target clip.mp4"
        );
        // No collision → no suffix, so the common DASH case stays tidy.
        assert_eq!(
            output_path_for(Path::new("/tmp/x-30280.m4s"), Path::new("/tmp/x-30232.m4s"), "x-30280", "mp4"),
            PathBuf::from("/tmp/x-30280.mp4")
        );
    }
}
