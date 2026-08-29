//! A single range connection: streams one chunk and writes it at the correct file
//! offset, honouring the speed limiter and pause/cancel control.

use crate::control::SharedControl;
use crate::limiter::CombinedLimiter;
use crate::model::ChunkState;
use std::path::Path;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("download canceled")]
    Canceled,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Download `chunk` of `url` into `file_path`, resuming from already-downloaded
/// bytes. `on_progress` is called with the number of bytes written per buffer so
/// the task can update shared counters.
pub async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    chunk: &ChunkState,
    file_path: &Path,
    limiter: &CombinedLimiter,
    control: &SharedControl,
    mut on_progress: impl FnMut(u64),
) -> Result<u64, ChunkError> {
    let start = chunk.resume_offset();
    let end = chunk.end;
    // `end == u64::MAX` means "to the end of the resource" (unknown size).
    let range = if end == u64::MAX {
        format!("bytes={}-", start)
    } else {
        format!("bytes={}-{}", start, end)
    };

    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, range)
        .send()
        .await?
        .error_for_status()?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(file_path)
        .await?;

    file.seek(std::io::SeekFrom::Start(start)).await?;

    let mut stream = resp.bytes_stream();
    let mut total_written: u64 = 0;

    use futures_util::StreamExt;
    while let Some(bytes) = stream.next().await {
        let bytes = bytes?;

        if control.is_canceled() {
            return Err(ChunkError::Canceled);
        }
        if control.is_paused() {
            if control.wait_while_paused().await {
                return Err(ChunkError::Canceled);
            }
        }

        // Throttle before writing this buffer.
        limiter.acquire(bytes.len() as u64).await;

        file.write_all(&bytes).await?;
        total_written += bytes.len() as u64;
        on_progress(bytes.len() as u64);
    }

    file.flush().await?;
    Ok(total_written)
}
