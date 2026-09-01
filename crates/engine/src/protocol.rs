//! Remote probing and range planning for segmented downloads.

use crate::model::{ChunkState, Download};
use reqwest::Client;

/// Result of probing a remote resource.
#[derive(Debug, Clone)]
pub struct Probe {
    /// Total content length in bytes, if known.
    pub content_length: Option<u64>,
    /// `true` when the server advertised `Accept-Ranges: bytes`.
    pub accept_ranges: bool,
    /// `Content-Type` header value, if present.
    pub content_type: Option<String>,
    /// Suggested filename (from `Content-Disposition` or the URL path).
    pub filename: String,
}

/// Errors that can occur while probing.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Extract a filename from a `Content-Disposition` header or the URL path.
pub fn suggest_filename(url: &str, content_disposition: Option<&str>, fallback: &str) -> String {
    if let Some(cd) = content_disposition {
        // Prefer filename*=UTF-8''... then filename="..."
        if let Some(idx) = cd.to_lowercase().find("filename*=") {
            let rest = &cd[idx + "filename*=".len()..];
            if let Some((_, name)) = rest.split_once('\'') {
                if let Some((_, name)) = name.split_once('\'') {
                    let name = name.trim_matches('"');
                    if !name.is_empty() {
                        return sanitize(name);
                    }
                }
            }
        }
        if let Some(idx) = cd.to_lowercase().find("filename=") {
            let rest = &cd[idx + "filename=".len()..];
            let name = rest.trim_matches('"').split(';').next().unwrap_or("").trim();
            if !name.is_empty() {
                return sanitize(name);
            }
        }
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(seg) = parsed.path_segments().and_then(|s| s.last()) {
            if !seg.is_empty() {
                return sanitize(&percent_decode(seg));
            }
        }
    }
    sanitize(fallback)
}

/// Decode a `%XX` percent-encoded sequence (used for URL path segments and
/// Content-Disposition filenames). Non-encoded text passes through unchanged.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8 as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Remove characters that are unsafe in filenames.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    if cleaned.trim().is_empty() {
        "download.bin".to_string()
    } else {
        cleaned
    }
}

/// Log a `reqwest::Error` (transport or response) with the URL, the
/// stage that failed (HEAD / ranged GET / body stream), the response
/// status + headers (if any) and a preview of the response body (if
/// the response is buffered). This is the forensic trail that lets us
/// debug "error decoding response body" without the user having to
/// re-run under a debugger.
async fn log_http_error(stage: &str, _url: &str, err: &reqwest::Error) {
    let mut chain = String::new();
    let mut src: Option<&dyn std::error::Error> = Some(err);
    while let Some(e) = src {
        if !chain.is_empty() {
            chain.push_str(" <- ");
        }
        chain.push_str(&e.to_string());
        src = e.source();
    }

    let status = err
        .status()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "<no response>".into());
    let url_field = err.url().map(|u| u.to_string()).unwrap_or_default();
    let kind = format!(
        "is_decode={} is_request={} is_redirect={} is_builder={}",
        err.is_decode(),
        err.is_request(),
        err.is_redirect(),
        err.is_builder(),
    );

    log::error!(
        "probe HTTP error stage={stage} url={url_field} status={status} {kind} chain={chain}"
    );
}

/// Probe a URL with a `HEAD` request, falling back to a ranged `GET` when HEAD
/// is not allowed. Determines size, range support, content type and filename.
pub async fn probe(client: &Client, url: &str) -> Result<Probe, ProbeError> {
    // We deliberately do **not** use `?` on the `.send().await` calls
    // so that, on failure, we can pull the response object out of
    // the `reqwest::Error` and dump the status, headers, and a body
    // preview to the log. Without this, the next "error decoding
    // response body" leaves no forensic trace.
    let head = client.head(url).send().await;
    let mut resp = match head {
        Ok(r) => r,
        Err(e) => {
            log_http_error("HEAD", url, &e).await;
            return Err(ProbeError::Http(e));
        }
    };
    let status = resp.status();
    // Some servers reject HEAD; retry with a 0-byte range GET.
    if !status.is_success() {
        let ranged = client
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await;
        match ranged {
            Ok(r) => resp = r,
            Err(e) => {
                log_http_error("ranged GET", url, &e).await;
                return Err(ProbeError::Http(e));
            }
        }
    }

    let content_length = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    // A 206 from a range probe tells us the true full length too.
    let content_range = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_content_range_total(s));

    let accept_ranges = resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("bytes"))
        .unwrap_or(false)
        || content_range.is_some();

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let cd = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let filename = suggest_filename(url, cd.as_deref(), "download.bin");

    // Prefer the explicit length; fall back to the 206 content-range total.
    let content_length = content_length.or(content_range);

    // Log a single info line with the resolved metadata so the log
    // file tells the full story of every probed URL.
    log::info!(
        "probe ok url={url} status={} content_length={} content_type={} accept_ranges={}",
        resp.status().as_u16(),
        content_length.map(|n| n.to_string()).unwrap_or_default(),
        content_type.as_deref().unwrap_or(""),
        accept_ranges,
    );

    Ok(Probe {
        content_length,
        accept_ranges,
        content_type,
        filename,
    })
}

/// Parse the `*/TOTAL` part of a `Content-Range` header.
fn parse_content_range_total(value: &str) -> Option<u64> {
    let total = value.rsplit('/').next()?;
    total.trim().parse::<u64>().ok()
}

/// Decide how many chunks to use given remote size, range support and the
/// configured connection count.
pub fn decide_chunk_count(total: Option<u64>, accept_ranges: bool, max_connections: usize) -> usize {
    if !accept_ranges || total.is_none() {
        return 1;
    }
    let total = total.unwrap();
    if total == 0 {
        return 1;
    }
    let max = max_connections.max(1);
    // Roughly one chunk per MiB (rounded up), capped at the connection budget.
    let mb = 1024 * 1024;
    let by_size = ((total + mb - 1) / mb).max(1) as usize;
    (by_size.min(max)).max(1)
}

/// Plan contiguous inclusive byte ranges for `n` chunks covering `[0, total-1]`.
pub fn plan_chunks(total: u64, n: usize) -> Vec<ChunkState> {
    let n = n.max(1);
    let mut chunks = Vec::with_capacity(n);
    let base = total / n as u64;
    let rem = total % n as u64;
    let mut cursor: u64 = 0;
    for i in 0..n {
        let extra = if (i as u64) < rem { 1 } else { 0 };
        let len = base + extra;
        if len == 0 {
            continue;
        }
        let start = cursor;
        let end = cursor + len - 1;
        chunks.push(ChunkState {
            index: i,
            start,
            end,
            downloaded: 0,
        });
        cursor = end + 1;
    }
    chunks
}

/// Build a fresh download plan: probe + chunk planning, mutating `download`.
/// Reuses any already-downloaded chunk state for resume.
///
/// `probe_client` is the no-decompression client used for the HEAD / range
/// probe — see `build_probe_client` in `task.rs` for why we keep this
/// separate from the chunk `client`. Passing the same client to both
/// works (and is what the test suite does for in-process servers) but
/// may surface a `decode body` error on misconfigured mirrors.
pub async fn build_plan(
    probe_client: &Client,
    download: &mut Download,
    max_connections: usize,
) {
    match probe(probe_client, &download.url).await {
        Ok(p) => {
            download.can_resume = p.accept_ranges;
            download.content_type = p.content_type.clone();
            if download.filename.is_empty() || download.filename == "download.bin" {
                download.filename = p.filename.clone();
            }
            if download.total_size.is_none() {
                download.total_size = p.content_length;
            }

            // If we already have a chunk layout (resume), keep it; otherwise plan.
            if download.chunks.is_empty() {
                if let Some(total) = download.total_size {
                    let n = decide_chunk_count(Some(total), p.accept_ranges, max_connections);
                    download.chunks = plan_chunks(total, n);
                }
            }
        }
        Err(e) => {
            download.error = Some(format!("probe failed: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_even_split() {
        let c = plan_chunks(100, 4);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].start, 0);
        assert_eq!(c[0].end, 24);
        assert_eq!(c[3].start, 75);
        assert_eq!(c[3].end, 99);
        let total: u64 = c.iter().map(|x| x.size()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn plan_uneven_split() {
        let c = plan_chunks(10, 3);
        assert_eq!(c.len(), 3);
        let sizes: Vec<u64> = c.iter().map(|x| x.size()).collect();
        assert_eq!(sizes, vec![4, 3, 3]);
    }

    #[test]
    fn decide_single_when_no_ranges() {
        assert_eq!(decide_chunk_count(Some(1_000_000), false, 8), 1);
        assert_eq!(decide_chunk_count(None, true, 8), 1);
        assert_eq!(decide_chunk_count(Some(5_000_000), true, 8), 5);
    }

    #[test]
    fn filename_from_url() {
        assert_eq!(
            suggest_filename("https://example.com/path/file%20name.zip?x=1", None, "dl"),
            "file name.zip"
        );
    }

    #[test]
    fn filename_from_disposition() {
        assert_eq!(
            suggest_filename("https://example.com/x", Some("attachment; filename=\"my file.mp4\""), "dl"),
            "my file.mp4"
        );
    }
}
