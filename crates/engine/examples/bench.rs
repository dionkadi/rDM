//! dm-engine 基准与压力测试 harness。
//!
//! 内置一个可注入故障的 tokio HTTP/1.1 测试服务器（Range/206、HEAD、限速、
//! 中途截断、无 Content-Length、404/403/500、HLS 播放列表含 AES-128），
//! 驱动真实的 DownloadManager 跑 10 组场景，全局采样 CPU / RSS / 磁盘 IO，
//! 结果写 JSON。
//!
//! 运行：cargo run --release -p dm-engine --example bench -- \
//!        --out target/bench/results.json [--scratch target/bench/tmp] [--quick] [--only a,b]

use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dm_engine::manager::{DownloadEvent, DownloadManager};
use dm_engine::model::{Download, DownloadStatus, ProxyMode, Settings};
use dm_engine::storage::Storage;
use dm_engine::task::sha256_of;

// ─────────────────────────────────────────────────────────────────────────────
// 字节模式：与引擎集成测试一致。byte(o) = ((o * 31) + 7) mod 256。
// 31 与 256 互质 → 周期 256 字节，服务器与校验端都可以按模板填充。
// ─────────────────────────────────────────────────────────────────────────────

fn pattern_template() -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, b) in t.iter_mut().enumerate() {
        *b = ((i as u64).wrapping_mul(31).wrapping_add(7) & 0xFF) as u8;
    }
    t
}

fn fill_pattern(buf: &mut [u8], off: u64) {
    let t = pattern_template();
    let start = (off % 256) as usize;
    let mut idx = 0usize;
    // 先对齐到 256 边界之前的部分
    let head = (256 - start).min(buf.len());
    for i in 0..head {
        buf[idx + i] = t[(start + i) % 256];
    }
    idx += head;
    // 之后每 256 字节整块拷贝
    while idx + 256 <= buf.len() {
        buf[idx..idx + 256].copy_from_slice(&t);
        idx += 256;
    }
    while idx < buf.len() {
        buf[idx] = t[(start + idx) % 256];
        idx += 1;
    }
}

fn verify_file(path: &Path, expected_len: u64) -> Result<(), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut expect = vec![0u8; 64 * 1024];
    let mut off = 0u64;
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        fill_pattern(&mut expect[..n], off);
        if buf[..n] != expect[..n] {
            // 找到第一个不匹配的字节
            for i in 0..n {
                if buf[i] != expect[i] {
                    return Err(format!(
                        "byte mismatch at offset {} (got {}, want {})",
                        off + i as u64,
                        buf[i],
                        expect[i]
                    ));
                }
            }
        }
        off += n as u64;
    }
    if off != expected_len {
        return Err(format!("length mismatch: got {off}, want {expected_len}"));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 测试服务器
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct HlsCfg {
    segments: usize,
    seg_size: usize,
    encrypted: bool,
}

#[derive(Clone)]
struct ServerCfg {
    total_size: u64,
    accept_ranges: bool,
    /// 不返回 Content-Length（HEAD/GET 都不带），模拟部分 CDN 代理
    omit_content_length: bool,
    /// 所有请求都返回该状态码（带一小段 HTML body）
    status_override: Option<u16>,
    /// 仅 HEAD 返回该状态码（GET 正常）
    head_status: Option<u16>,
    /// 无 Referer 的请求一律 403（防盗链，含 HEAD）
    require_referer: bool,
    /// 每连接限速 bytes/s（0 = 不限）
    throttle_per_conn: u64,
    /// 发送这么多 body 字节后直接断开连接（模拟中途断连）
    truncate_after: Option<u64>,
    /// 发送响应头之前的延迟
    first_byte_delay_ms: u64,
    hls: Option<HlsCfg>,
    /// 服务器自身端口（HLS 播放列表生成绝对 URL 用）
    self_port: u16,
}

impl Default for ServerCfg {
    fn default() -> Self {
        ServerCfg {
            total_size: 0,
            accept_ranges: true,
            omit_content_length: false,
            status_override: None,
            head_status: None,
            require_referer: false,
            throttle_per_conn: 0,
            truncate_after: None,
            first_byte_delay_ms: 0,
            hls: None,
            self_port: 0,
        }
    }
}

#[derive(Default)]
struct ServerStats {
    connections: AtomicU64,
    requests: AtomicU64,
    bytes_sent: AtomicU64,
    errors: AtomicU64,
}

struct ServerHandle {
    port: u16,
    stats: Arc<ServerStats>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn start_server(cfg: ServerCfg) -> ServerHandle {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    let mut cfg = cfg;
    cfg.self_port = port;
    let stats = Arc::new(ServerStats::default());
    let stats2 = Arc::clone(&stats);
    let accept_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    stats2.connections.fetch_add(1, Ordering::Relaxed);
                    let cfg = cfg.clone();
                    let st = Arc::clone(&stats2);
                    tokio::spawn(async move {
                        handle_conn(stream, cfg, st).await;
                    });
                }
                Err(_) => break,
            }
        }
    });
    ServerHandle {
        port,
        stats,
        accept_task,
    }
}

async fn read_request(s: &mut tokio::net::TcpStream) -> Option<(String, String, String)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 || Instant::now() > deadline {
            return None;
        }
        let mut tmp = [0u8; 2048];
        let n = match tokio::io::AsyncReadExt::read(s, &mut tmp).await {
            Ok(0) => return None,
            Ok(n) => n,
            Err(_) => return None,
        };
        buf.extend_from_slice(&tmp[..n]);
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let mut lines = text.split("\r\n");
    let req_line = lines.next()?.to_string();
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let headers = lines.collect::<Vec<_>>().join("\r\n");
    Some((method, path, headers))
}

fn header_val(headers: &str, name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_ascii_lowercase() == lower {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn parse_range(spec: &str, total: u64) -> Option<(u64, u64)> {
    let inner = spec.trim_start_matches("bytes=").trim();
    let mut it = inner.splitn(2, '-');
    let start = it.next()?.trim().parse::<u64>().ok()?;
    let end = match it.next() {
        Some("") | None => total.saturating_sub(1),
        Some(e) => e.trim().parse::<u64>().ok()?,
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end.min(total - 1)))
}

async fn handle_conn(mut s: tokio::net::TcpStream, cfg: ServerCfg, stats: Arc<ServerStats>) {
    let _ = s.set_nodelay(true);
    loop {
        let Some((method, path, headers)) = read_request(&mut s).await else {
            break;
        };
        stats.requests.fetch_add(1, Ordering::Relaxed);
        let req_close = header_val(&headers, "connection")
            .map(|v| v.eq_ignore_ascii_case("close"))
            .unwrap_or(false);
        let close = match serve_request(&mut s, &method, &path, &headers, &cfg, &stats).await {
            Ok(c) => c || req_close,
            Err(_) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                true
            }
        };
        if close {
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut s).await;
            break;
        }
    }
}

/// 返回 Ok(是否应关闭连接)
async fn serve_request(
    s: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    headers: &str,
    cfg: &ServerCfg,
    stats: &ServerStats,
) -> std::io::Result<bool> {
    // 通用错误码注入
    if let Some(code) = cfg.status_override {
        let body = format!("<html>HTTP {code}</html>");
        let resp = format!(
            "HTTP/1.1 {code} Error\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        return Ok(true);
    }

    // HLS 路由
    if cfg.hls.is_some() && path.starts_with("/hls/") {
        return serve_hls(s, path, cfg, stats).await;
    }

    let is_head = method.eq_ignore_ascii_case("HEAD");
    let total = cfg.total_size;

    // 防盗链（真实 CDN 对 HEAD 同样 403）
    if cfg.require_referer && header_val(headers, "referer").is_none() {
        let body = "<html>403 forbidden</html>";
        let resp = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        return Ok(true);
    }

    if is_head {
        if let Some(code) = cfg.head_status {
            let resp = format!(
                "HTTP/1.1 {code} Not Allowed\r\nContent-Type: text/html\r\nContent-Length: 17\r\nConnection: keep-alive\r\n\r\n"
            );
            tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
            return Ok(false);
        }
        let mut resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n");
        if !cfg.omit_content_length {
            resp.push_str(&format!("Content-Length: {total}\r\n"));
        }
        resp.push_str(&format!(
            "Accept-Ranges: {}\r\nConnection: keep-alive\r\n\r\n",
            if cfg.accept_ranges { "bytes" } else { "none" }
        ));
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        return Ok(false);
    }

    // 防盗链已在上方统一处理（含 HEAD）。

    if cfg.first_byte_delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(cfg.first_byte_delay_ms)).await;
    }

    // 空文件：忽略 Range，直接 200 + CL 0
    if total == 0 {
        let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nAccept-Ranges: bytes\r\nConnection: keep-alive\r\n\r\n";
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        return Ok(false);
    }

    let range = header_val(headers, "range").and_then(|v| parse_range(&v, total));

    if cfg.accept_ranges && !cfg.omit_content_length {
        if let Some((start, end)) = range {
            let len = end - start + 1;
            let resp = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nConnection: keep-alive\r\n\r\n"
            );
            tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
            write_body(s, start, len, cfg, stats).await?;
            return Ok(false);
        }
    }

    // 200 整文件
    if cfg.omit_content_length {
        // 无 CL：必须用连接关闭分隔 body
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
        );
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        write_body(s, 0, total, cfg, stats).await?;
        return Ok(true);
    }

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nConnection: keep-alive\r\n\r\n"
    );
    tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
    write_body(s, 0, total, cfg, stats).await?;
    Ok(false)
}

async fn write_body(
    s: &mut tokio::net::TcpStream,
    mut off: u64,
    mut len: u64,
    cfg: &ServerCfg,
    stats: &ServerStats,
) -> std::io::Result<()> {
    const BLOCK: usize = 64 * 1024;
    let mut buf = vec![0u8; BLOCK];
    let mut sent: u64 = 0;
    while len > 0 {
        let n = len.min(BLOCK as u64) as usize;
        fill_pattern(&mut buf[..n], off);
        tokio::io::AsyncWriteExt::write_all(s, &buf[..n]).await?;
        off += n as u64;
        len -= n as u64;
        sent += n as u64;
        stats.bytes_sent.fetch_add(n as u64, Ordering::Relaxed);
        if let Some(tr) = cfg.truncate_after {
            if sent >= tr {
                // 中途断连
                let _ = tokio::io::AsyncWriteExt::shutdown(s).await;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "truncated on purpose",
                ));
            }
        }
        if cfg.throttle_per_conn > 0 {
            let secs = n as f64 / cfg.throttle_per_conn as f64;
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
    }
    Ok(())
}

async fn serve_hls(
    s: &mut tokio::net::TcpStream,
    path: &str,
    cfg: &ServerCfg,
    stats: &ServerStats,
) -> std::io::Result<bool> {
    let hls = cfg.hls.as_ref().unwrap();
    let base = format!("http://127.0.0.1:{}", cfg.self_port);
    if path.ends_with("index.m3u8") {
        let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXT-X-VERSION:3\n");
        if hls.encrypted {
            text.push_str(&format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{base}/hls/key.bin\",IV=0x000102030405060708090a0b0c0d0e0f\n"));
        }
        text.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
        for i in 0..hls.segments {
            text.push_str(&format!("#EXTINF:1.0,\n{base}/hls/seg_{i}.ts\n"));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
            text.len(),
            text
        );
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        return Ok(false);
    }
    if path.ends_with("key.bin") {
        let key: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: 16\r\nContent-Type: application/octet-stream\r\nConnection: keep-alive\r\n\r\n"
        );
        tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
        tokio::io::AsyncWriteExt::write_all(s, &key).await?;
        return Ok(false);
    }
    if let Some(idx) = path.rfind("/seg_") {
        let num_str = &path[idx + 5..path.len().saturating_sub(3)]; // strip "/seg_" prefix and ".ts"
        let n: usize = num_str.parse().unwrap_or(usize::MAX);
        if n < hls.segments {
            let plain_len = hls.seg_size;
            if hls.encrypted {
                use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
                type Enc = cbc::Encryptor<aes::Aes128>;
                let key: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
                let iv: [u8; 16] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f];
                let mut plain = vec![0u8; plain_len];
                fill_pattern(&mut plain, (n as u64) * plain_len as u64);
                let mut ct_buf = vec![0u8; plain_len + 16];
                let ct_len = Enc::new_from_slices(&key, &iv)
                    .expect("key/iv")
                    .encrypt_padded_mut::<Pkcs7>(&mut ct_buf, plain_len)
                    .expect("encrypt")
                    .len();
                ct_buf.truncate(ct_len);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: video/mp2t\r\nConnection: keep-alive\r\n\r\n",
                    ct_buf.len()
                );
                tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
                tokio::io::AsyncWriteExt::write_all(s, &ct_buf).await?;
                stats.bytes_sent.fetch_add(ct_buf.len() as u64, Ordering::Relaxed);
            } else {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {plain_len}\r\nContent-Type: video/mp2t\r\nConnection: keep-alive\r\n\r\n"
                );
                tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
                let mut buf = vec![0u8; 64 * 1024];
                let mut off = (n as u64) * plain_len as u64;
                let mut remaining = plain_len as u64;
                while remaining > 0 {
                    let take = remaining.min(buf.len() as u64) as usize;
                    fill_pattern(&mut buf[..take], off);
                    tokio::io::AsyncWriteExt::write_all(s, &buf[..take]).await?;
                    off += take as u64;
                    remaining -= take as u64;
                    stats.bytes_sent.fetch_add(take as u64, Ordering::Relaxed);
                }
            }
            return Ok(false);
        }
    }
    let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(s, resp.as_bytes()).await?;
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// 资源采样（/proc）
// ─────────────────────────────────────────────────────────────────────────────

fn read_proc_snapshot() -> (u64, u64, u64, u64) {
    // (cpu_ms, rss_kb, read_bytes, write_bytes)
    let mut cpu_ms = 0u64;
    if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
        // field 14 utime, 15 stime（1-based，且 comm 可能含空格 → 以 ')' 为界）
        if let Some(idx) = stat.rfind(')') {
            let rest = &stat[idx + 2..];
            let fields: Vec<&str> = rest.split_whitespace().collect();
            // rest 从 field 3 (state) 开始 → utime 是 rest 的第 11 个字段（索引 11），stime 索引 12
            if fields.len() >= 13 {
                let utime: u64 = fields[11].parse().unwrap_or(0);
                let stime: u64 = fields[12].parse().unwrap_or(0);
                cpu_ms = (utime + stime) * 10; // CLK_TCK = 100
            }
        }
    }
    let mut rss_kb = 0u64;
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                rss_kb = v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
            }
        }
    }
    let mut rb = 0u64;
    let mut wb = 0u64;
    if let Ok(io) = std::fs::read_to_string("/proc/self/io") {
        for line in io.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") {
                rb = v.trim().parse().unwrap_or(0);
            }
            if let Some(v) = line.strip_prefix("write_bytes:") {
                wb = v.trim().parse().unwrap_or(0);
            }
        }
    }
    (cpu_ms, rss_kb, rb, wb)
}

#[allow(dead_code)]
struct ResSample {
    t: Instant,
    cpu_ms: u64,
    rss_kb: u64,
    read_bytes: u64,
    write_bytes: u64,
}

/// 全局采样线程：每 500ms 记录一次（用于 max RSS 与 soak 的全局兜底）
fn spawn_monitor(samples: Arc<Mutex<Vec<ResSample>>>, stop: Arc<AtomicU64>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while stop.load(Ordering::Relaxed) == 0 {
            let (cpu, rss, rb, wb) = read_proc_snapshot();
            samples.lock().unwrap().push(ResSample {
                t: Instant::now(),
                cpu_ms: cpu,
                rss_kb: rss,
                read_bytes: rb,
                write_bytes: wb,
            });
            std::thread::sleep(Duration::from_millis(500));
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 事件计数
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct EventCounts {
    added: AtomicU64,
    progress: AtomicU64,
    status_changed: AtomicU64,
    completed: AtomicU64,
    error: AtomicU64,
    removed: AtomicU64,
}

fn event_sink(counts: Arc<EventCounts>) -> dm_engine::manager::EventSink {
    Arc::new(move |e| match e {
        DownloadEvent::Added(_) => {
            counts.added.fetch_add(1, Ordering::Relaxed);
        }
        DownloadEvent::Progress(_) => {
            counts.progress.fetch_add(1, Ordering::Relaxed);
        }
        DownloadEvent::StatusChanged(_) => {
            counts.status_changed.fetch_add(1, Ordering::Relaxed);
        }
        DownloadEvent::Completed(_) => {
            counts.completed.fetch_add(1, Ordering::Relaxed);
        }
        DownloadEvent::Error(_) => {
            counts.error.fetch_add(1, Ordering::Relaxed);
        }
        DownloadEvent::Removed(_) => {
            counts.removed.fetch_add(1, Ordering::Relaxed);
        }
    })
}

fn event_snapshot(c: &EventCounts) -> serde_json::Value {
    serde_json::json!({
        "added": c.added.load(Ordering::Relaxed),
        "progress": c.progress.load(Ordering::Relaxed),
        "status_changed": c.status_changed.load(Ordering::Relaxed),
        "completed": c.completed.load(Ordering::Relaxed),
        "error": c.error.load(Ordering::Relaxed),
        "removed": c.removed.load(Ordering::Relaxed),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Bench 框架
// ─────────────────────────────────────────────────────────────────────────────

struct Bench {
    scratch: PathBuf,
    results: Vec<serde_json::Value>,
    events: Arc<EventCounts>,
    quick: bool,
    seq: AtomicU64,
}

impl Bench {
    fn new(scratch: PathBuf, quick: bool) -> Self {
        std::fs::create_dir_all(&scratch).ok();
        Bench {
            scratch,
            results: Vec::new(),
            events: Arc::new(EventCounts::default()),
            quick,
            seq: AtomicU64::new(0),
        }
    }

    fn record(&mut self, name: &str, objective: &str, params: serde_json::Value, metrics: serde_json::Value, notes: Vec<String>) {
        println!("── {name} 完成");
        self.results.push(serde_json::json!({
            "name": name,
            "objective": objective,
            "params": params,
            "metrics": metrics,
            "notes": notes,
        }));
    }

    fn next_id(&self) -> String {
        format!("bench-{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }

    fn settings(&self, max_dl: usize, conns: usize, global_limit: Option<u64>) -> Settings {
        Settings {
            max_concurrent_downloads: max_dl,
            connections_per_download: conns,
            default_directory: self.scratch.clone(),
            proxy_mode: ProxyMode::None,
            speed_limit_global: global_limit,
            clipboard_monitor: false,
            ..Settings::default()
        }
    }

    async fn new_manager(&self, db_name: &str, max_dl: usize, conns: usize, global_limit: Option<u64>) -> (DownloadManager, PathBuf) {
        let db = self.scratch.join(format!("{db_name}.sqlite"));
        let _ = std::fs::remove_file(&db);
        let storage = Storage::open(&db).expect("open storage");
        let mgr = DownloadManager::with_settings(storage, self.settings(max_dl, conns, global_limit));
        mgr.set_event_sink(event_sink(Arc::clone(&self.events)));
        (mgr, db)
    }

    fn make_download(&self, url: String, save_path: PathBuf) -> Download {
        let mut d = Download::new(url);
        d.id = self.next_id();
        d.save_path = save_path;
        d
    }
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((p / 100.0) * (values.len() as f64 - 1.0)).round() as usize;
    values[idx.min(values.len() - 1)]
}

async fn wait_terminal(mgr: &DownloadManager, id: &str, timeout: Duration) -> Option<Download> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(d) = mgr.get(id) {
            if matches!(
                d.status,
                DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
            ) {
                return Some(d);
            }
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

async fn wait_min_bytes(mgr: &DownloadManager, id: &str, min_bytes: u64, timeout: Duration) -> Option<Download> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(d) = mgr.get(id) {
            if d.downloaded >= min_bytes {
                return Some(d);
            }
            if matches!(
                d.status,
                DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
            ) {
                return Some(d);
            }
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

#[allow(dead_code)]
struct ResDelta {
    wall: Duration,
    cpu_ms: u64,
    avg_cores: f64,
    rss_max_kb: u64,
    rss_end_kb: u64,
}

fn res_delta(samples: &Mutex<Vec<ResSample>>, start_idx: usize, wall: Duration, cpu0: u64) -> (usize, ResDelta) {
    let all = samples.lock().unwrap();
    let end_idx = all.len();
    let mut cpu = 0u64;
    let mut rss_max = 0u64;
    let mut rss_end = 0u64;
    for s in &all[start_idx..] {
        rss_max = rss_max.max(s.rss_kb);
        rss_end = s.rss_kb;
    }
    if let Some(last) = all.last() {
        cpu = last.cpu_ms.saturating_sub(cpu0);
    }
    let delta = ResDelta {
        wall,
        cpu_ms: cpu,
        avg_cores: if wall.as_millis() > 0 {
            cpu as f64 / wall.as_millis() as f64
        } else {
            0.0
        },
        rss_max_kb: rss_max,
        rss_end_kb: rss_end,
    };
    (end_idx, delta)
}

// 简化版：场景级 IO 差分直接用 read_proc_snapshot
struct IoMark {
    rb: u64,
    wb: u64,
}

fn io_mark() -> IoMark {
    let (_, _, rb, wb) = read_proc_snapshot();
    IoMark { rb, wb }
}

fn io_delta_mb(m0: &IoMark) -> (f64, f64) {
    let (_, _, rb, wb) = read_proc_snapshot();
    (
        rb.saturating_sub(m0.rb) as f64 / (1024.0 * 1024.0),
        wb.saturating_sub(m0.wb) as f64 / (1024.0 * 1024.0),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 1：probe 延迟
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_probe_latency(bench: &mut Bench) {
    let cfg = ServerCfg {
        total_size: 256 * 1024,
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/probe.bin", server.port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let n = 100;
    let mut lat = Vec::with_capacity(n);
    let mut errs = 0usize;
    for _ in 0..n {
        let t0 = Instant::now();
        match dm_engine::protocol::probe(&client, &url, &BTreeMap::new(), None).await {
            Ok(_) => lat.push(t0.elapsed().as_secs_f64() * 1000.0),
            Err(_) => errs += 1,
        }
    }
    let p50 = percentile(&mut lat, 50.0);
    let p95 = percentile(&mut lat, 95.0);
    let p99 = percentile(&mut lat, 99.0);
    bench.record(
        "probe_latency",
        "URL 探测（HEAD/Range 探测）单次延迟分布，反映添加下载的响应速度",
        serde_json::json!({"n": n, "url": url}),
        serde_json::json!({
            "p50_ms": (p50 * 1000.0).round() / 1000.0,
            "p95_ms": (p95 * 1000.0).round() / 1000.0,
            "p99_ms": (p99 * 1000.0).round() / 1000.0,
            "max_ms": (lat.iter().cloned().fold(0.0f64, f64::max) * 1000.0).round() / 1000.0,
            "errors": errs,
        }),
        vec!["本机回环地址，无网络 RTT；生产环境需叠加真实 RTT 与 TLS 握手".to_string()],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 2：单文件多连接吞吐（chunk scaling）
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_chunk_scaling(bench: &mut Bench, samples: &Arc<Mutex<Vec<ResSample>>>) {
    let size: u64 = if bench.quick { 32 * 1024 * 1024 } else { 128 * 1024 * 1024 };
    let cfg = ServerCfg {
        total_size: size,
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/big.bin", server.port);
    let mut rows = Vec::new();
    let mut notes = Vec::new();

    for conns in [1usize, 2, 4, 8, 16, 32] {
        let (mgr, _db) = bench.new_manager("chunk_scaling", 1, conns, None).await;
        let out = bench.scratch.join(format!("chunk_{conns}.bin"));
        let _ = std::fs::remove_file(&out);
        let start_idx = samples.lock().unwrap().len();
        let io0 = io_mark();
        let cpu0 = read_proc_snapshot().0;
        let d = bench.make_download(url.clone(), out.clone());
        let id = d.id.clone();
        let t0 = Instant::now();
        mgr.add(d);
        let res = wait_terminal(&mgr, &id, Duration::from_secs(120)).await;
        let wall = t0.elapsed();
        let (_eidx, rdelta) = res_delta(samples, start_idx, wall, cpu0);
        let (read_mb, write_mb) = io_delta_mb(&io0);
        match res {
            Some(d) if d.status == DownloadStatus::Completed => {
                let tp = size as f64 / (1024.0 * 1024.0) / wall.as_secs_f64();
                let v = verify_file(&out, size);
                let verify_ok = v.is_ok();
                let verify_err = v.err();
                rows.push(serde_json::json!({
                    "connections": conns,
                    "wall_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
                    "throughput_mb_s": (tp * 1000.0).round() / 1000.0,
                    "verify_ok": verify_ok,
                    "verify_err": verify_err,
                    "avg_cpu_cores": (rdelta.avg_cores * 1000.0).round() / 1000.0,
                    "write_io_mb": (write_mb * 1000.0).round() / 1000.0,
                    "read_io_mb": (read_mb * 1000.0).round() / 1000.0,
                }));
                println!("  conns={conns}: {tp:.1} MB/s, verify={verify_ok}");
            }
            Some(d) => {
                rows.push(serde_json::json!({
                    "connections": conns,
                    "status": format!("{:?}", d.status),
                    "error": d.error,
                }));
                notes.push(format!("conns={conns} 未完成: {:?}", d.error));
            }
            None => {
                rows.push(serde_json::json!({"connections": conns, "status": "timeout"}));
                notes.push(format!("conns={conns} 超时"));
            }
        }
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        let _ = std::fs::remove_file(&_db);
    }
    bench.record(
        "chunk_scaling",
        "单文件分块下载的吞吐随连接数的扩展性，同时观察 CPU/IO 成本与字节正确性",
        serde_json::json!({"payload_mb": size / (1024 * 1024), "connections_tested": [1,2,4,8,16,32], "max_concurrent_downloads": 1}),
        serde_json::json!({"rows": rows}),
        notes,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 3：并发下载扩展性
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_concurrency(bench: &mut Bench, samples: &Arc<Mutex<Vec<ResSample>>>) {
    let size: u64 = if bench.quick { 4 * 1024 * 1024 } else { 8 * 1024 * 1024 };
    let cfg = ServerCfg {
        total_size: size,
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/c.bin", server.port);
    let mut rows = Vec::new();
    let mut notes = Vec::new();

    for n in [5usize, 10, 25, 50, 100] {
        let (mgr, _db) = bench.new_manager("concurrency", n, 4, None).await;
        let start_idx = samples.lock().unwrap().len();
        let io0 = io_mark();
        let cpu0 = read_proc_snapshot().0;
        let ev0 = event_snapshot(&bench.events);
        let mut ids = Vec::new();
        let t0 = Instant::now();
        for i in 0..n {
            let out = bench.scratch.join(format!("conc_{n}_{i}.bin"));
            let d = bench.make_download(url.clone(), out.clone());
            ids.push((d.id.clone(), out.clone()));
            mgr.add(d);
        }
        let mut lat = Vec::new();
        let mut errors = Vec::new();
        let mut completed = 0usize;
        for (id, out) in &ids {
            match wait_terminal(&mgr, id, Duration::from_secs(180)).await {
                Some(d) => {
                    if d.status == DownloadStatus::Completed {
                        completed += 1;
                        lat.push(t0.elapsed().as_secs_f64());
                        let _ = std::fs::remove_file(out);
                    } else {
                        errors.push(format!("{}: {:?}", d.error.clone().unwrap_or_default(), d.status));
                    }
                }
                None => errors.push("timeout".to_string()),
            }
        }
        let wall = t0.elapsed();
        let (_eidx, rdelta) = res_delta(samples, start_idx, wall, cpu0);
        let (read_mb, write_mb) = io_delta_mb(&io0);
        let ev1 = event_snapshot(&bench.events);
        let total_mb = (completed as f64) * size as f64 / (1024.0 * 1024.0);
        let agg_tp = total_mb / wall.as_secs_f64();
        lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rows.push(serde_json::json!({
            "concurrent_downloads": n,
            "completed": completed,
            "errors": errors,
            "makespan_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "aggregate_throughput_mb_s": (agg_tp * 1000.0).round() / 1000.0,
            "per_download_latency_s": {
                "p50": lat.get(lat.len()/2).copied().unwrap_or(0.0),
                "p95": lat.get((lat.len() as f64 * 0.95) as usize % lat.len().max(1)).copied().unwrap_or(0.0),
                "max": lat.last().copied().unwrap_or(0.0),
            },
            "avg_cpu_cores": (rdelta.avg_cores * 1000.0).round() / 1000.0,
            "write_io_mb": (write_mb * 1000.0).round() / 1000.0,
            "read_io_mb": (read_mb * 1000.0).round() / 1000.0,
            "events_delta": {
                "progress": ev1["progress"].as_u64().unwrap_or(0) - ev0["progress"].as_u64().unwrap_or(0),
                "status_changed": ev1["status_changed"].as_u64().unwrap_or(0) - ev0["status_changed"].as_u64().unwrap_or(0),
            },
        }));
        println!("  n={n}: {agg_tp:.1} MB/s 聚合, {completed}/{n} 完成, CPU {:.2} 核", rdelta.avg_cores);
        if !errors.is_empty() {
            notes.push(format!("n={n} 有 {} 个失败", errors.len()));
        }
        // 清理可能的残留 .part
        for i in 0..n {
            let _ = std::fs::remove_file(bench.scratch.join(format!("conc_{n}_{i}.bin")));
            let _ = std::fs::remove_file(bench.scratch.join(format!("conc_{n}_{i}.bin.part")));
        }
        let _ = std::fs::remove_file(&_db);
    }
    bench.record(
        "concurrency_scaling",
        "并发下载数从 5 到 100 的聚合吞吐、单下载延迟分布、错误率与 CPU 成本",
        serde_json::json!({"payload_mb_per_dl": size / (1024 * 1024), "connections_per_download": 4, "levels": [5,10,25,50,100]}),
        serde_json::json!({"rows": rows}),
        notes,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 4：调度器排队（max_concurrent=3, 50 个排队）
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_scheduler_queue(bench: &mut Bench) {
    let size: u64 = 4 * 1024 * 1024;
    let cfg = ServerCfg {
        total_size: size,
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/q.bin", server.port);
    let (mgr, _db) = bench.new_manager("queue", 3, 4, None).await;
    let n = 50usize;
    let mut ids = Vec::new();
    let t0 = Instant::now();
    for i in 0..n {
        let out = bench.scratch.join(format!("q_{i}.bin"));
        let d = bench.make_download(url.clone(), out.clone());
        ids.push((d.id.clone(), out.clone()));
        mgr.add(d);
    }
    // 记录前 3 个立即进入 Downloading 的时刻
    let mut first_terminal: Option<Duration> = None;
    let mut done = 0usize;
    let mut errors = 0usize;
    for (id, out) in &ids {
        match wait_terminal(&mgr, id, Duration::from_secs(180)).await {
            Some(d) => {
                if first_terminal.is_none() {
                    first_terminal = Some(t0.elapsed());
                }
                if d.status == DownloadStatus::Completed {
                    done += 1;
                } else {
                    errors += 1;
                }
                let _ = std::fs::remove_file(out);
                let _ = std::fs::remove_file(out.with_extension("bin.part"));
            }
            None => errors += 1,
        }
    }
    let makespan = t0.elapsed();
    let agg = (done as f64 * size as f64 / (1024.0 * 1024.0)) / makespan.as_secs_f64();
    let _ = first_terminal;
    bench.record(
        "scheduler_queue",
        "槽位上限（3）下 50 个下载的排队调度行为：makespan、错误率",
        serde_json::json!({"max_concurrent_downloads": 3, "n": n, "payload_mb": size / (1024 * 1024)}),
        serde_json::json!({
            "completed": done,
            "errors": errors,
            "makespan_s": (makespan.as_secs_f64() * 1000.0).round() / 1000.0,
            "aggregate_throughput_mb_s": (agg * 1000.0).round() / 1000.0,
        }),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 5：限速精度
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_rate_limit(bench: &mut Bench) {
    let size: u64 = 8 * 1024 * 1024;
    let mut rows = Vec::new();
    let mut notes = Vec::new();

    // (a) 全局限速 8MB/s，4 个并发下载（合计 32MB）
    {
        let cfg = ServerCfg { total_size: size, ..Default::default() };
        let server = start_server(cfg).await;
        let url = format!("http://127.0.0.1:{}/r.bin", server.port);
        let (mgr, _db) = bench.new_manager("rate_global", 4, 4, Some(8 * 1024 * 1024)).await;
        let mut ids = Vec::new();
        let t0 = Instant::now();
        for i in 0..4 {
            let d = bench.make_download(url.clone(), bench.scratch.join(format!("rg_{i}.bin")));
            ids.push((d.id.clone(), bench.scratch.join(format!("rg_{i}.bin"))));
            mgr.add(d);
        }
        let mut ok = 0usize;
        let mut errs = Vec::new();
        for (id, out) in &ids {
            match wait_terminal(&mgr, id, Duration::from_secs(60)).await {
                Some(d) if d.status == DownloadStatus::Completed => ok += 1,
                Some(d) => errs.push(d.error.unwrap_or_default()),
                None => errs.push("timeout".into()),
            }
            let _ = std::fs::remove_file(out);
            let _ = std::fs::remove_file(out.with_extension("bin.part"));
        }
        let wall = t0.elapsed();
        let achieved = if ok == 4 { 32.0 / wall.as_secs_f64() } else { 0.0 };
        rows.push(serde_json::json!({
            "mode": "global 8 MB/s × 4 downloads",
            "target_mb_s": 8.0,
            "achieved_mb_s": (achieved * 1000.0).round() / 1000.0,
            "wall_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "completed": ok,
            "errors": errs,
            "error_pct": if ok == 4 { serde_json::json!(((achieved - 8.0) / 8.0 * 100.0 * 10.0).round() / 10.0) } else { serde_json::json!(null) },
        }));
        let _ = std::fs::remove_file(&_db);
    }

    // (b) 单下载限速 2MB/s × 4 并发（全局不限）
    {
        let cfg = ServerCfg { total_size: size, ..Default::default() };
        let server = start_server(cfg).await;
        let url = format!("http://127.0.0.1:{}/r2.bin", server.port);
        let (mgr, _db) = bench.new_manager("rate_per_dl", 4, 4, None).await;
        let mut ids = Vec::new();
        let t0 = Instant::now();
        for i in 0..4 {
            let mut d = bench.make_download(url.clone(), bench.scratch.join(format!("rp_{i}.bin")));
            d.speed_limit = Some(2 * 1024 * 1024);
            ids.push((d.id.clone(), bench.scratch.join(format!("rp_{i}.bin"))));
            mgr.add(d);
        }
        let mut ok = 0usize;
        let mut errs = Vec::new();
        for (id, out) in &ids {
            match wait_terminal(&mgr, id, Duration::from_secs(60)).await {
                Some(d) if d.status == DownloadStatus::Completed => ok += 1,
                Some(d) => errs.push(d.error.unwrap_or_default()),
                None => errs.push("timeout".into()),
            }
            let _ = std::fs::remove_file(out);
            let _ = std::fs::remove_file(out.with_extension("bin.part"));
        }
        let wall = t0.elapsed();
        let achieved = if ok == 4 { 32.0 / wall.as_secs_f64() } else { 0.0 };
        rows.push(serde_json::json!({
            "mode": "per-download 2 MB/s × 4 downloads",
            "target_mb_s": 8.0,
            "achieved_mb_s": (achieved * 1000.0).round() / 1000.0,
            "wall_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "completed": ok,
            "errors": errs,
            "error_pct": if ok == 4 { serde_json::json!(((achieved - 8.0) / 8.0 * 100.0 * 10.0).round() / 10.0) } else { serde_json::json!(null) },
        }));
        let _ = std::fs::remove_file(&_db);
    }

    notes.push("TokenBucket 突发容量 = rate × 20%，短下载会略微超速（突发预支）".to_string());
    bench.record(
        "rate_limiting",
        "全局与单下载限速的精度：目标 8MB/s 下的实际达成率与误差",
        serde_json::json!({"payload_mb_each": 8, "parallel": 4}),
        serde_json::json!({"rows": rows}),
        notes,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 6：边界与异常
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_edge_cases(bench: &mut Bench) {
    let mut rows = Vec::new();

    async fn run_one(
        bench: &Bench,
        label: &str,
        expect: &str,
        cfg: ServerCfg,
        modify: impl Fn(&mut Download),
        port_override: Option<u16>,
        conns: usize,
    ) -> serde_json::Value {
        let server = if port_override.is_none() {
            Some(start_server(cfg.clone()).await)
        } else {
            None
        };
        let port = port_override.or_else(|| server.as_ref().map(|s| s.port)).unwrap();
        let url = format!("http://127.0.0.1:{port}/edge.bin");
        let (mgr, _db) = bench.new_manager("edge", 2, conns, None).await;
        let out = bench.scratch.join(format!("edge_{label}.bin"));
        let _ = std::fs::remove_file(&out);
        let mut d = bench.make_download(url.clone(), out.clone());
        modify(&mut d);
        let id = d.id.clone();
        let t0 = Instant::now();
        mgr.add(d);
        let res = wait_terminal(&mgr, &id, Duration::from_secs(90)).await;
        let detect = t0.elapsed();
        let r = match res {
            Some(d) => {
                let status_ok = match expect {
                    "completed" => d.status == DownloadStatus::Completed,
                    "error" => d.status == DownloadStatus::Error,
                    "canceled" => d.status == DownloadStatus::Canceled,
                    _ => true,
                };
                serde_json::json!({
                    "case": label,
                    "expect": expect,
                    "status": format!("{:?}", d.status),
                    "match": status_ok,
                    "detect_ms": (detect.as_secs_f64() * 1000.0).round() / 1000.0,
                    "error": d.error,
                    "total_size": d.total_size,
                    "downloaded": d.downloaded,
                })
            }
            None => serde_json::json!({
                "case": label,
                "expect": expect,
                "status": "TIMEOUT(90s)",
                "match": false,
            }),
        };
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        drop(server);
        let _ = std::fs::remove_file(&_db);
        println!("  edge {label}: {}", serde_json::to_string(&r).unwrap_or_default());
        r
    }

    // 404 / 500：全部请求返回错误码
    rows.push(run_one(bench, "http_404", "error", ServerCfg { total_size: 1024, status_override: Some(404), ..Default::default() }, |_| {}, None, 4).await);
    rows.push(run_one(bench, "http_500", "error", ServerCfg { total_size: 1024, status_override: Some(500), ..Default::default() }, |_| {}, None, 4).await);

    // 连接拒绝：先占住端口再释放
    let dead_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    rows.push(run_one(bench, "conn_refused", "error", ServerCfg::default(), |_| {}, Some(dead_port), 4).await);

    // 中途截断：发送一半后断连
    rows.push(run_one(
        bench,
        "truncated_body",
        "error",
        ServerCfg { total_size: 8 * 1024 * 1024, truncate_after: Some(4 * 1024 * 1024), ..Default::default() },
        |_| {},
        None,
        1,
    ).await);

    // 无 Content-Length + 小 body（<1KB）→ open-ended 截断保护
    rows.push(run_one(
        bench,
        "no_length_tiny_body",
        "error",
        ServerCfg { total_size: 500, omit_content_length: true, ..Default::default() },
        |_| {},
        None,
        4,
    ).await);

    // 无 Content-Length + 正常 body → open-ended 完成
    rows.push(run_one(
        bench,
        "no_length_full",
        "completed",
        ServerCfg { total_size: 16 * 1024 * 1024, omit_content_length: true, ..Default::default() },
        |_| {},
        None,
        4,
    ).await);

    // HEAD 405 → 退化为 Range 探测
    rows.push(run_one(
        bench,
        "head_405",
        "completed",
        ServerCfg { total_size: 4 * 1024 * 1024, head_status: Some(405), ..Default::default() },
        |_| {},
        None,
        4,
    ).await);

    // 不支持 Range → 单连接回退
    rows.push(run_one(
        bench,
        "no_ranges",
        "completed",
        ServerCfg { total_size: 4 * 1024 * 1024, accept_ranges: false, ..Default::default() },
        |_| {},
        None,
        4,
    ).await);

    // 0 字节文件
    rows.push(run_one(bench, "zero_byte", "completed", ServerCfg { total_size: 0, ..Default::default() }, |_| {}, None, 4).await);

    // 慢服务器：单连接 512KB/s，4MB → 预期 ~8s
    {
        let r = run_one(
            bench,
            "slow_server_512k",
            "completed",
            ServerCfg { total_size: 4 * 1024 * 1024, throttle_per_conn: 512 * 1024, ..Default::default() },
            |d| {
                d.speed_limit = None;
            },
            None,
            1,
        ).await;
        let mut r = r;
        r["expected_wall_s"] = serde_json::json!(8.0);
        rows.push(r);
    }

    // 防盗链：无 Referer 必须 403；有 Referer 必须完成
    rows.push(run_one(
        bench,
        "anti_leech_no_referer",
        "error",
        ServerCfg { total_size: 1024 * 1024, require_referer: true, ..Default::default() },
        |_| {},
        None,
        4,
    ).await);
    rows.push(run_one(
        bench,
        "anti_leech_with_referer",
        "completed",
        ServerCfg { total_size: 1024 * 1024, require_referer: true, ..Default::default() },
        |d| {
            d.headers.insert("Referer".into(), "https://example.com/".into());
        },
        None,
        4,
    ).await);

    // 错误 checksum
    rows.push(run_one(
        bench,
        "checksum_mismatch",
        "error",
        ServerCfg { total_size: 1024 * 1024, ..Default::default() },
        |d| {
            d.checksum = Some(dm_engine::model::ChecksumSpec {
                algorithm: "sha256".into(),
                expected: "deadbeef".repeat(8),
            });
        },
        None,
        4,
    ).await);

    // 暂停/恢复：限速保证中间态可捕获
    {
        let cfg = ServerCfg {
            total_size: 16 * 1024 * 1024,
            throttle_per_conn: 2 * 1024 * 1024,
            ..Default::default()
        };
        let server = start_server(cfg).await;
        let url = format!("http://127.0.0.1:{}/pr.bin", server.port);
        let (mgr, _db) = bench.new_manager("edge_pr", 1, 8, None).await;
        let out = bench.scratch.join("edge_pr.bin");
        let _ = std::fs::remove_file(&out);
        let d = bench.make_download(url, out.clone());
        let id = d.id.clone();
        mgr.add(d);
        let mut row = serde_json::json!({"case": "pause_resume_mid_transfer"});
        if let Some(_snap) = wait_min_bytes(&mgr, &id, 2 * 1024 * 1024, Duration::from_secs(30)).await {
            let t_pause = Instant::now();
            mgr.pause(&id);
            // 等状态稳定为 Paused
            let mut waited = 0u64;
            loop {
                match mgr.get(&id) {
                    Some(d) if d.status == DownloadStatus::Paused => break,
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
                waited += 15;
                if waited > 5000 {
                    break;
                }
            }
            let paused_at = mgr.get(&id).map(|d| d.downloaded).unwrap_or(0);
            tokio::time::sleep(Duration::from_millis(1000)).await;
            let after_1s = mgr.get(&id).map(|d| d.downloaded).unwrap_or(0);
            mgr.resume(&id);
            let res = wait_terminal(&mgr, &id, Duration::from_secs(60)).await;
            let verify = res
                .as_ref()
                .filter(|d| d.status == DownloadStatus::Completed)
                .map(|_| verify_file(&out, 16 * 1024 * 1024));
            row["status"] = serde_json::json!(res.as_ref().map(|d| format!("{:?}", d.status)).unwrap_or_default());
            row["paused_at_bytes"] = serde_json::json!(paused_at);
            row["frozen_after_1s"] = serde_json::json!(after_1s == paused_at);
            row["resume_to_complete_s"] = serde_json::json!((t_pause.elapsed().as_secs_f64() * 1000.0).round() / 1000.0);
            row["verify_ok"] = serde_json::json!(verify.map(|v| v.is_ok()).unwrap_or(false));
            row["bytes_lost_on_resume"] = serde_json::json!(res.as_ref().map(|d| paused_at.saturating_sub(d.downloaded.min(paused_at))).unwrap_or(0));
        } else {
            row["status"] = serde_json::json!("never reached 2MB (transfer too fast?)");
        }
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        let _ = std::fs::remove_file(&_db);
        println!("  edge pause_resume: {}", serde_json::to_string(&row).unwrap_or_default());
        rows.push(row);
    }

    // 取消：限速保证中间态
    {
        let cfg = ServerCfg {
            total_size: 16 * 1024 * 1024,
            throttle_per_conn: 2 * 1024 * 1024,
            ..Default::default()
        };
        let server = start_server(cfg).await;
        let url = format!("http://127.0.0.1:{}/cc.bin", server.port);
        let (mgr, _db) = bench.new_manager("edge_cc", 1, 8, None).await;
        let out = bench.scratch.join("edge_cc.bin");
        let _ = std::fs::remove_file(&out);
        let d = bench.make_download(url, out.clone());
        let id = d.id.clone();
        mgr.add(d);
        let mut row = serde_json::json!({"case": "cancel_mid_transfer"});
        if let Some(snap) = wait_min_bytes(&mgr, &id, 2 * 1024 * 1024, Duration::from_secs(30)).await {
            let partial = snap.downloaded;
            let t0 = Instant::now();
            mgr.cancel(&id);
            let res = wait_terminal(&mgr, &id, Duration::from_secs(30)).await;
            row["status"] = serde_json::json!(res.as_ref().map(|d| format!("{:?}", d.status)).unwrap_or_else(|| "timeout".to_string()));
            row["cancel_detect_ms"] = serde_json::json!((t0.elapsed().as_secs_f64() * 1000.0).round() / 1000.0);
            row["partial_bytes"] = serde_json::json!(partial);
            row["progress_not_lost"] = serde_json::json!(res.as_ref().map(|d| d.downloaded >= partial).unwrap_or(false));
        }
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        let _ = std::fs::remove_file(&_db);
        println!("  edge cancel: {}", serde_json::to_string(&row).unwrap_or_default());
        rows.push(row);
    }

    bench.record(
        "edge_cases",
        "13 类边界/异常场景：错误码、断连、无长度、HEAD 拒绝、慢速、防盗链、校验和、暂停恢复、取消",
        serde_json::json!({"payload_note": "各场景独立服务器与独立下载"}),
        serde_json::json!({"rows": rows}),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 7：小文件高周转
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_small_files(bench: &mut Bench) {
    let size: u64 = 256 * 1024;
    let cfg = ServerCfg { total_size: size, ..Default::default() };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/s.bin", server.port);
    let (mgr, _db) = bench.new_manager("small_files", 3, 2, None).await;
    let n = 200usize;
    let mut ids = Vec::new();
    let t0 = Instant::now();
    for i in 0..n {
        let d = bench.make_download(url.clone(), bench.scratch.join(format!("s_{i}.bin")));
        ids.push((d.id.clone(), bench.scratch.join(format!("s_{i}.bin"))));
        mgr.add(d);
    }
    let mut lat = Vec::new();
    let mut completed = 0usize;
    let mut errors = 0usize;
    for (id, out) in &ids {
        match wait_terminal(&mgr, id, Duration::from_secs(120)).await {
            Some(d) => {
                if d.status == DownloadStatus::Completed {
                    completed += 1;
                    lat.push(t0.elapsed().as_secs_f64() * 1000.0);
                } else {
                    errors += 1;
                }
            }
            None => errors += 1,
        }
        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
    }
    let wall = t0.elapsed();
    let p50 = percentile(&mut lat, 50.0);
    let p95 = percentile(&mut lat, 95.0);
    bench.record(
        "small_files_throughput",
        "200 个 256KB 小文件在 3 并发槽下的周转率与端到端延迟（add → 终态）",
        serde_json::json!({"n": n, "payload_kb": 256, "max_concurrent_downloads": 3}),
        serde_json::json!({
            "completed": completed,
            "errors": errors,
            "makespan_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "per_file_throughput_per_s": ((completed as f64) / wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "latency_ms": { "p50": (p50 * 1000.0).round() / 1000.0, "p95": (p95 * 1000.0).round() / 1000.0 },
            "aggregate_mb_s": ((completed as f64 * size as f64 / (1024.0*1024.0)) / wall.as_secs_f64() * 1000.0).round() / 1000.0,
        }),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 8：存储层
// ─────────────────────────────────────────────────────────────────────────────

fn make_row(id: &str, total: u64, downloaded: u64, chunks: usize) -> Download {
    let mut d = Download::new(format!("https://example.com/{id}.bin"));
    d.id = id.to_string();
    d.filename = format!("{id}.bin");
    d.save_path = PathBuf::from(format!("/tmp/dm-bench/{id}.bin"));
    d.total_size = Some(total);
    d.downloaded = downloaded;
    d.status = DownloadStatus::Completed;
    d.chunks = (0..chunks)
        .map(|i| dm_engine::model::ChunkState {
            index: i,
            start: (total / chunks as u64) * i as u64,
            end: (total / chunks as u64) * (i as u64 + 1) - 1,
            downloaded: total / chunks as u64,
        })
        .collect();
    d
}

async fn scenario_storage(bench: &mut Bench) {
    let db_path = bench.scratch.join("storage_bench.sqlite");
    let _ = std::fs::remove_file(&db_path);
    let storage = Storage::open(&db_path).expect("storage");

    // 10k 不同行 insert（含 8 chunk JSON）
    let n = 10_000usize;
    let t0 = Instant::now();
    for i in 0..n {
        let d = make_row(&format!("row-{i}"), 64 * 1024 * 1024, 64 * 1024 * 1024, 8);
        if let Err(e) = storage.save_download(&d) {
            bench.record("storage_bench", "存储层", serde_json::json!({}), serde_json::json!({"error": e.to_string()}), vec![]);
            return;
        }
    }
    let insert_wall = t0.elapsed();

    // 2k 次 upsert 同一行（模拟 progress flush 热更新）
    let mut hot = make_row("hot-row", 1024 * 1024 * 1024, 0, 8);
    let t0 = Instant::now();
    for i in 1..=2000u64 {
        hot.downloaded = i * 512 * 1024;
        hot.chunks[0].downloaded = hot.downloaded.min(128 * 1024 * 1024);
        let _ = storage.save_download(&hot);
    }
    let upsert_wall = t0.elapsed();

    // load_active（10k 行全部是 Completed，active 应该很少）→ 改测全表 load
    // load_active 只读 active 状态行；为了测大数据量读取，插入 1000 行 active
    let t0 = Instant::now();
    for i in 0..1000 {
        let mut d = make_row(&format!("act-{i}"), 64 * 1024 * 1024, 32 * 1024 * 1024, 8);
        d.status = DownloadStatus::Downloading;
        let _ = storage.save_download(&d);
    }
    let insert_active_wall = t0.elapsed();
    let t0 = Instant::now();
    let active = storage.load_active().unwrap_or_default();
    let load_active_wall = t0.elapsed();

    // history 写入 10k
    let t0 = Instant::now();
    for i in 0..n {
        let _ = storage.add_history(
            Some(&format!("row-{i}")),
            &format!("https://example.com/{i}.bin"),
            &format!("{i}.bin"),
            Some(64 * 1024 * 1024),
            "2026-11-11T00:00:00Z",
            None,
            None,
        );
    }
    let history_wall = t0.elapsed();

    // manager 层 list()：10k Error 行（不 spawn）
    let (mgr, mgr_db) = bench.new_manager("storage_list", 3, 4, None).await;
    let t0 = Instant::now();
    for i in 0..3000 {
        let mut d = make_row(&format!("m-{i}"), 1024 * 1024, 0, 1);
        d.status = DownloadStatus::Error;
        d.error = Some("probe failed".into());
        mgr.add(d);
    }
    let manager_add_wall = t0.elapsed();
    let mut list_walls = Vec::new();
    for _ in 0..5 {
        let t0 = Instant::now();
        let l = mgr.list();
        list_walls.push(t0.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(l.len(), 3000);
    }
    let p50_list = percentile(&mut list_walls, 50.0);

    // reorder 1000 ids
    let ids: Vec<String> = (0..1000).map(|i| format!("m-{}", 2900 + i)).collect();
    let t0 = Instant::now();
    mgr.reorder(&ids);
    let reorder_wall = t0.elapsed();

    let db_size_mb = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0) as f64 / (1024.0 * 1024.0);

    bench.record(
        "storage_bench",
        "SQLite 存储层：批量插入、热更新 upsert、active 读取、history 写入、manager.list() 与 reorder 的成本",
        serde_json::json!({"rows": n, "chunk_json_per_row": 8, "history_rows": n}),
        serde_json::json!({
            "insert_10k_ms": insert_wall.as_millis(),
            "insert_10k_ops_per_s": ((n as f64) / insert_wall.as_secs_f64() * 10.0).round() / 10.0,
            "hot_upsert_2k_ms": upsert_wall.as_millis(),
            "hot_upsert_ops_per_s": ((2000.0) / upsert_wall.as_secs_f64() * 10.0).round() / 10.0,
            "insert_1k_active_ms": insert_active_wall.as_millis(),
            "load_active_1k_ms": load_active_wall.as_millis(),
            "load_active_rows": active.len(),
            "history_10k_ms": history_wall.as_millis(),
            "history_10k_ops_per_s": ((n as f64) / history_wall.as_secs_f64() * 10.0).round() / 10.0,
            "manager_add_3k_ms": manager_add_wall.as_millis(),
            "manager_list_3k_p50_ms": (p50_list * 1000.0).round() / 1000.0,
            "reorder_1k_ms": reorder_wall.as_millis(),
            "db_size_mb": (db_size_mb * 1000.0).round() / 1000.0,
        }),
        vec!["save_download 为逐行 UPSERT，每次写事务；无批量事务包装".to_string()],
    );
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(mgr_db);
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 9：HLS
// ─────────────────────────────────────────────────────────────────────────────

async fn run_hls_once(bench: &Bench, encrypted: bool, segments: usize, seg_size: usize) -> serde_json::Value {
    let cfg = ServerCfg {
        total_size: 0,
        hls: Some(HlsCfg { segments, seg_size, encrypted }),
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/hls/index.m3u8", server.port);
    let (mgr, _db) = bench.new_manager("hls", 1, 8, None).await;
    let d = bench.make_download(url, bench.scratch.join("hls_out.bin"));
    let id = d.id.clone();
    let t0 = Instant::now();
    mgr.add(d);
    let res = wait_terminal(&mgr, &id, Duration::from_secs(120)).await;
    let wall = t0.elapsed();
    let mut row = serde_json::json!({ "encrypted": encrypted, "segments": segments, "seg_kb": seg_size / 1024 });
    match res {
        Some(d) if d.status == DownloadStatus::Completed => {
            let final_path = d.save_path.clone();
            let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
            let expected = (segments * seg_size) as u64;
            row["status"] = serde_json::json!("completed");
            row["wall_s"] = serde_json::json!((wall.as_secs_f64() * 1000.0).round() / 1000.0);
            row["mb_s"] = serde_json::json!((bytes as f64 / (1024.0*1024.0) / wall.as_secs_f64() * 1000.0).round() / 1000.0);
            row["segments_per_s"] = serde_json::json!((segments as f64 / wall.as_secs_f64() * 1000.0).round() / 1000.0);
            row["final_filename"] = serde_json::json!(d.filename);
            row["bytes"] = serde_json::json!(bytes);
            row["size_matches_plain_sum"] = serde_json::json!(bytes == expected);
            row["remux_attempted"] = serde_json::json!(d.filename.ends_with(".mp4"));
        }
        Some(d) => {
            row["status"] = serde_json::json!(format!("{:?}", d.status));
            row["error"] = serde_json::json!(d.error);
        }
        None => {
            row["status"] = serde_json::json!("timeout");
        }
    }
    let _ = std::fs::remove_file(bench.scratch.join("hls_out.ts"));
    let _ = std::fs::remove_file(bench.scratch.join("hls_out.mp4"));
    let _ = std::fs::remove_file(bench.scratch.join("download.ts"));
    let _ = std::fs::remove_file(bench.scratch.join("download.ts.part"));
    let _ = std::fs::remove_file(&_db);
    row
}

async fn scenario_hls(bench: &mut Bench) {
    let (segments, seg_size) = if bench.quick { (40, 256 * 1024) } else { (100, 512 * 1024) };
    let plain = run_hls_once(bench, false, segments, seg_size).await;
    let enc = run_hls_once(bench, true, segments, seg_size).await;
    let ffmpeg = std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    bench.record(
        "hls_download",
        "HLS 播放列表下载：分段并发拉取与顺序拼接的吞吐；AES-128 加密流的解密开销；ffmpeg remux 可用性",
        serde_json::json!({"segments": segments, "seg_kb": seg_size / 1024, "concurrency_cap": 8}),
        serde_json::json!({ "plain": plain, "encrypted": enc, "ffmpeg_available": ffmpeg }),
        vec![
            "HLS 并发上限硬编码 8（HLS_MAX_PARALLEL）".to_string(),
            "每完成一个段就写一次 SQLite（save_download），段数多时 DB 写入放大".to_string(),
        ],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 10：SHA-256 校验吞吐
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_sha256(bench: &mut Bench) {
    let size: u64 = if bench.quick { 64 * 1024 * 1024 } else { 256 * 1024 * 1024 };
    let path = bench.scratch.join("sha.bin");
    {
        let mut f = std::fs::File::create(&path).expect("create");
        let mut buf = vec![0u8; 1024 * 1024];
        let mut off = 0u64;
        while off < size {
            let n = ((size - off).min(buf.len() as u64)) as usize;
            fill_pattern(&mut buf[..n], off);
            f.write_all(&buf[..n]).unwrap();
            off += n as u64;
        }
    }
    let t0 = Instant::now();
    let digest = sha256_of(&path).expect("sha");
    let wall = t0.elapsed();
    let tp = size as f64 / (1024.0 * 1024.0) / wall.as_secs_f64();
    let _ = std::fs::remove_file(&path);
    bench.record(
        "sha256_verify",
        "完成时 SHA-256 整文件校验的吞吐（决定大文件完成后的尾部延迟）",
        serde_json::json!({"payload_mb": size / (1024 * 1024)}),
        serde_json::json!({
            "wall_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
            "throughput_mb_s": (tp * 1000.0).round() / 1000.0,
            "digest": digest,
        }),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 11：长时间稳定性（soak）
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_soak(bench: &mut Bench) {
    let dur = if bench.quick { 40u64 } else { 120u64 };
    let size: u64 = 8 * 1024 * 1024;
    let cfg = ServerCfg {
        total_size: size,
        throttle_per_conn: 4 * 1024 * 1024,
        ..Default::default()
    };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/soak.bin", server.port);
    let (mgr, db) = bench.new_manager("soak", 4, 4, None).await;
    let ev0 = event_snapshot(&bench.events);

    let mut series: Vec<serde_json::Value> = Vec::new();
    let mut completed = 0u64;
    let mut failed = 0u64;
    let mut canceled = 0u64;
    let mut failures: Vec<String> = Vec::new();

    let mut active: Vec<String> = Vec::new();
    let mut seq = 0usize;
    let mut paused_pending: Vec<(String, Instant)> = Vec::new();
    let mut next_churn = Instant::now() + Duration::from_secs(15);
    let mut churn_round = 0u64;

    let start = Instant::now();
    let bytes0 = server.stats.bytes_sent.load(Ordering::Relaxed);
    let (cpu0, _, _, _) = read_proc_snapshot();
    let rss_start = read_proc_snapshot().1;

    // 初始填充
    for _ in 0..4 {
        let out = bench.scratch.join(format!("soak_{seq}.bin"));
        let d = bench.make_download(url.clone(), out.clone());
        active.push(d.id.clone());
        mgr.add(d);
        seq += 1;
    }

    let mut last_sample = Instant::now();

    while start.elapsed().as_secs() < dur {
        // 采样（每 2s）
        if last_sample.elapsed() >= Duration::from_secs(2) {
            last_sample = Instant::now();
            let (cpu, rss, _, _) = read_proc_snapshot();
            let b = server.stats.bytes_sent.load(Ordering::Relaxed);
            series.push(serde_json::json!({
                "t_s": (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
                "rss_mb": (rss as f64 / 1024.0 * 100.0).round() / 100.0,
                "cpu_cores": ((cpu.saturating_sub(cpu0)) as f64 / (start.elapsed().as_millis() as f64) * 100.0).round() / 100.0,
                "mb_sent_total": (b as f64 / (1024.0*1024.0) * 10.0).round() / 10.0,
            }));
        }

        // 处理待恢复
        let now = Instant::now();
        for (id, deadline) in paused_pending.clone() {
            if now >= deadline {
                mgr.resume(&id);
                paused_pending.retain(|(i, _)| i != &id);
            }
        }

        // churn
        if now >= next_churn && !active.is_empty() {
            next_churn = now + Duration::from_secs(15);
            churn_round += 1;
            let idx = (churn_round as usize) % active.len();
            let id = active[idx].clone();
            if churn_round % 2 == 1 {
                mgr.pause(&id);
                paused_pending.push((id, now + Duration::from_secs(2)));
            } else {
                // cancel 并替换
                mgr.cancel(&id);
                canceled += 1;
                if let Some(d) = mgr.get(&id) {
                    let _ = std::fs::remove_file(&d.save_path);
                    let _ = std::fs::remove_file(d.part_path());
                }
                mgr.remove(&id);
                active.retain(|x| x != &id);
            }
        }

        // 回收终态 + 补位
        let mut replacement = Vec::new();
        active.retain(|id| {
            match mgr.get(id) {
                Some(d) if matches!(d.status, DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled) => {
                    match d.status {
                        DownloadStatus::Completed => completed += 1,
                        DownloadStatus::Error => {
                            failed += 1;
                            failures.push(format!("soak error: {:?}", d.error));
                        }
                        DownloadStatus::Canceled => canceled += 1,
                        _ => {}
                    }
                    let _ = std::fs::remove_file(&d.save_path);
                    let _ = std::fs::remove_file(d.part_path());
                    mgr.remove(id);
                    // 补一个新的
                    let out = bench.scratch.join(format!("soak_{seq}.bin"));
                    let nd = bench.make_download(url.clone(), out.clone());
                    replacement.push(nd.id.clone());
                    mgr.add(nd);
                    seq += 1;
                    false
                }
                _ => true,
            }
        });
        active.extend(replacement);

        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // 收尾：取消剩余
    for id in active.clone() {
        mgr.cancel(&id);
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    for id in active.clone() {
        if let Some(d) = mgr.get(&id) {
            let _ = std::fs::remove_file(&d.save_path);
            let _ = std::fs::remove_file(d.part_path());
        }
        mgr.remove(&id);
    }

    let wall = start.elapsed();
    let bytes1 = server.stats.bytes_sent.load(Ordering::Relaxed);
    let (cpu1, rss_end, _, _) = read_proc_snapshot();
    let total_mb = (bytes1.saturating_sub(bytes0)) as f64 / (1024.0 * 1024.0);

    // 用 series 重建 10s 窗口吞吐（每 5 个采样 = 10s）
    let mut fine_windows = Vec::new();
    for c in series.chunks(5) {
        if c.len() < 2 {
            continue;
        }
        let b0 = c[0]["mb_sent_total"].as_f64().unwrap_or(0.0);
        let b1 = c.last().unwrap()["mb_sent_total"].as_f64().unwrap_or(0.0);
        let t0 = c[0]["t_s"].as_f64().unwrap_or(0.0);
        let t1 = c.last().unwrap()["t_s"].as_f64().unwrap_or(0.0);
        let dt = t1 - t0;
        if dt > 0.0 {
            fine_windows.push(((b1 - b0) / dt * 100.0).round() / 100.0);
        }
    }

    // RSS 线性回归斜率（MB/min）
    let n = series.len() as f64;
    let mut slope = 0.0;
    if n > 4.0 {
        let mut sx = 0.0;
        let mut sy = 0.0;
        let mut sxy = 0.0;
        let mut sxx = 0.0;
        for s in &series {
            let x = s["t_s"].as_f64().unwrap_or(0.0) / 60.0;
            let y = s["rss_mb"].as_f64().unwrap_or(0.0);
            sx += x;
            sy += y;
            sxy += x * y;
            sxx += x * x;
        }
        let denom = n * sxx - sx * sx;
        if denom.abs() > 1e-9 {
            slope = (n * sxy - sx * sy) / denom;
        }
    }
    let rss_series: Vec<f64> = series
        .iter()
        .map(|s| s["rss_mb"].as_f64().unwrap_or(0.0))
        .collect();
    let rss_min = rss_series.iter().cloned().fold(f64::MAX, f64::min);
    let rss_max = rss_series.iter().cloned().fold(0.0f64, f64::max);

    // 残留 .part 文件
    let mut leftover_parts = 0usize;
    if let Ok(rd) = std::fs::read_dir(&bench.scratch) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x == "part").unwrap_or(false) {
                leftover_parts += 1;
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    let ev1 = event_snapshot(&bench.events);
    bench.record(
        "soak_stability",
        "持续负载下的稳定性：保持 4 个并发下载的完成-补位循环 + 周期性 pause/resume 与 cancel churn，观察内存趋势、吞吐稳定性与错误率",
        serde_json::json!({
            "duration_s": dur,
            "payload_mb": size / (1024*1024),
            "server_throttle_per_conn_mb_s": 4,
            "max_concurrent_downloads": 4,
            "churn_every_s": 15,
        }),
        serde_json::json!({
            "wall_s": (wall.as_secs_f64() * 10.0).round() / 10.0,
            "completed": completed,
            "canceled": canceled,
            "failed": failed,
            "failures": failures.iter().take(5).collect::<Vec<_>>(),
            "total_mb_transferred": (total_mb * 10.0).round() / 10.0,
            "avg_throughput_mb_s": ((total_mb / wall.as_secs_f64()) * 10.0).round() / 10.0,
            "ten_s_window_throughput_mb_s": { "series": fine_windows, "min": fine_windows.iter().cloned().fold(f64::MAX, f64::min), "max": fine_windows.iter().cloned().fold(0.0f64, f64::max) },
            "rss_mb": { "start": (rss_start as f64/1024.0*100.0).round()/100.0, "end": (rss_end as f64/1024.0*100.0).round()/100.0, "min": (rss_min*100.0).round()/100.0, "max": (rss_max*100.0).round()/100.0 },
            "rss_leak_slope_mb_per_min": (slope * 100.0).round() / 100.0,
            "avg_cpu_cores": ((cpu1.saturating_sub(cpu0)) as f64 / wall.as_millis() as f64 * 100.0).round() / 100.0,
            "events": { "delta": { "progress": ev1["progress"].as_u64().unwrap_or(0) - ev0["progress"].as_u64().unwrap_or(0), "status_changed": ev1["status_changed"].as_u64().unwrap_or(0) - ev0["status_changed"].as_u64().unwrap_or(0), "completed": ev1["completed"].as_u64().unwrap_or(0) - ev0["completed"].as_u64().unwrap_or(0) } },
            "sample_series": series,
            "leftover_part_files_cleaned": leftover_parts,
        }),
        vec![],
    );
    let _ = std::fs::remove_file(&db);
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 12：存储层对单流吞吐的影响（文件 DB vs 内存 DB 对照）
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_storage_impact(bench: &mut Bench) {
    let size: u64 = 32 * 1024 * 1024;
    let cfg = ServerCfg { total_size: size, ..Default::default() };
    let server = start_server(cfg).await;
    let url = format!("http://127.0.0.1:{}/si.bin", server.port);
    let mut rows = Vec::new();

    for mode in ["file-db", "memory-db"] {
        let out = bench.scratch.join(format!("si_{mode}.bin"));
        let _ = std::fs::remove_file(&out);
        let storage = if mode == "file-db" {
            let db = bench.scratch.join("storage_impact.sqlite");
            let _ = std::fs::remove_file(&db);
            Storage::open(&db).expect("file storage")
        } else {
            Storage::open_memory().expect("memory storage")
        };
        let mgr = DownloadManager::with_settings(storage, bench.settings(1, 1, None));
        mgr.set_event_sink(event_sink(Arc::clone(&bench.events)));
        let d = bench.make_download(url.clone(), out.clone());
        let id = d.id.clone();
        let t0 = Instant::now();
        mgr.add(d);
        let res = wait_terminal(&mgr, &id, Duration::from_secs(60)).await;
        let wall = t0.elapsed();
        if let Some(d) = res {
            if d.status == DownloadStatus::Completed {
                let tp = size as f64 / (1024.0 * 1024.0) / wall.as_secs_f64();
                rows.push(serde_json::json!({
                    "mode": mode,
                    "throughput_mb_s": (tp * 1000.0).round() / 1000.0,
                    "wall_s": (wall.as_secs_f64() * 1000.0).round() / 1000.0,
                }));
                println!("  storage_impact {mode}: {tp:.1} MB/s");
            }
        }
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
    }

    bench.record(
        "storage_impact",
        "对照实验：文件 SQLite（逐行事务 fsync）与内存 SQLite 下的单连接吞吐差异，用于确认进度 flush 对单流吞吐的封顶作用",
        serde_json::json!({"payload_mb": 32, "connections": 1}),
        serde_json::json!({"rows": rows}),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 场景 13：修复验证（proxy 绕过 + mirror 回退）
// ─────────────────────────────────────────────────────────────────────────────

async fn scenario_fix_verification(bench: &mut Bench) {
    let size: u64 = 4 * 1024 * 1024;
    let mut rows = Vec::new();

    // (a) ProxyMode::None + 恶意 env 代理：修复前 reqwest 会把流量送进
    // 死代理导致 502；修复后 build_client 无代理参数时显式 no_proxy，
    // 必须直连成功。
    {
        let cfg = ServerCfg { total_size: size, ..Default::default() };
        let server = start_server(cfg).await;
        let dead_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let p = l.local_addr().unwrap().port();
            drop(l);
            p
        };
        // 指向死端口的“恶意”代理——任何经过它的请求都会失败
        for k in ["HTTP_PROXY", "http_proxy"] {
            std::env::set_var(k, format!("http://127.0.0.1:{dead_port}"));
        }
        let url = format!("http://127.0.0.1:{}/pb.bin", server.port);
        let (mgr, _db) = bench.new_manager("fix_proxy", 1, 4, None).await;
        let out = bench.scratch.join("fix_proxy.bin");
        let _ = std::fs::remove_file(&out);
        let d = bench.make_download(url, out.clone());
        let id = d.id.clone();
        mgr.add(d);
        let res = wait_terminal(&mgr, &id, Duration::from_secs(30)).await;
        let status = res.as_ref().map(|d| format!("{:?}", d.status)).unwrap_or_default();
        let err = res.as_ref().and_then(|d| d.error.clone());
        let verify = res
            .as_ref()
            .filter(|d| d.status == DownloadStatus::Completed)
            .map(|d| verify_file(&out, size).is_ok())
            .unwrap_or(false);
        for k in ["HTTP_PROXY", "http_proxy"] {
            std::env::remove_var(k);
        }
        rows.push(serde_json::json!({
            "case": "proxy_none_bypasses_env",
            "status": status,
            "error": err,
            "verify_ok": verify,
            "expect": "Completed（直连成功，绕过指向死端口的 env 代理）",
        }));
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        let _ = std::fs::remove_file(&_db);
    }

    // (b) mirror 回退：主 URL 无监听，镜像可用 → 必须完成且主 URL 被替换。
    {
        let cfg = ServerCfg { total_size: size, ..Default::default() };
        let server = start_server(cfg).await;
        let dead_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let p = l.local_addr().unwrap().port();
            drop(l);
            p
        };
        let primary = format!("http://127.0.0.1:{dead_port}/dead.bin");
        let mirror = format!("http://127.0.0.1:{}/mirror.bin", server.port);
        let (mgr, _db) = bench.new_manager("fix_mirror", 1, 4, None).await;
        let out = bench.scratch.join("fix_mirror.bin");
        let _ = std::fs::remove_file(&out);
        let mut d = bench.make_download(primary, out.clone());
        d.mirrors = vec![mirror.clone()];
        let id = d.id.clone();
        let promoted_to_mirror = false;
        mgr.add(d);
        let res = wait_terminal(&mgr, &id, Duration::from_secs(30)).await;
        let status = res.as_ref().map(|d| format!("{:?}", d.status)).unwrap_or_default();
        let err = res.as_ref().and_then(|d| d.error.clone());
        let final_url = res.as_ref().map(|d| d.url.clone()).unwrap_or_default();
        let verify = res
            .as_ref()
            .filter(|d| d.status == DownloadStatus::Completed)
            .map(|d| verify_file(&out, size).is_ok())
            .unwrap_or(false);
        let _ = promoted_to_mirror;
        rows.push(serde_json::json!({
            "case": "mirror_fallback_on_dead_primary",
            "status": status,
            "error": err,
            "final_url": final_url,
            "promoted_to_mirror": final_url == mirror,
            "verify_ok": verify,
            "expect": "Completed 且 url 被替换为镜像",
        }));
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("bin.part"));
        let _ = std::fs::remove_file(&_db);
    }

    bench.record(
        "fix_verification",
        "修复验证：ProxyMode::None 必须绕过环境代理直连；主 URL 探测失败必须回退到可用镜像并升格为主 URL",
        serde_json::json!({"payload_mb": size / (1024 * 1024)}),
        serde_json::json!({"rows": rows}),
        vec![],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// main
// ─────────────────────────────────────────────────────────────────────────────

fn env_info() -> serde_json::Value {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        })
        .unwrap_or_default();
    let mem_total_kb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines().next().map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            })
        })
        .unwrap_or(0);
    let hostname = std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim().to_string();
    serde_json::json!({
        "engine_version": dm_engine::VERSION,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "os": std::env::var("uname").unwrap_or_else(|_| {
            std::fs::read_to_string("/proc/version").unwrap_or_default().trim().to_string()
        }),
        "hostname": hostname,
        "cpu_model": cpu_model,
        "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "mem_total_gb": (mem_total_kb as f64 / (1024.0*1024.0) * 10.0).round() / 10.0,
        "started_at": chrono::Utc::now().to_rfc3339(),
    })
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut out_path = PathBuf::from("target/bench/results.json");
    let mut scratch = PathBuf::from("target/bench/tmp");
    let mut quick = false;
    let mut only: Option<Vec<String>> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_path = PathBuf::from(&args[i]);
            }
            "--scratch" => {
                i += 1;
                scratch = PathBuf::from(&args[i]);
            }
            "--quick" => quick = true,
            "--only" => {
                i += 1;
                only = Some(args[i].split(',').map(|s| s.trim().to_string()).collect());
            }
            other => {
                eprintln!("未知参数 {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // 关键：reqwest 默认读取系统代理环境变量（HTTP_PROXY/HTTPS_PROXY），
    // 本机若配了代理会把所有回环流量转发出去，污染全部吞吐与延迟数据。
    // 引擎的 ProxyMode::None 只影响 settings 层，不会调用 reqwest 的
    // .no_proxy()，因此这里必须在进程级清掉代理变量。
    for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::remove_var(k);
    }

    let samples = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicU64::new(0));
    let monitor = spawn_monitor(Arc::clone(&samples), Arc::clone(&stop));

    let mut bench = Bench::new(scratch.clone(), quick);
    let env = env_info();
    println!("DM engine benchmark — engine v{} ({})", dm_engine::VERSION, env["profile"]);
    println!("scratch = {}", bench.scratch.display());

    let run = |name: &str, only: &Option<Vec<String>>| {
        only.as_ref().map(|o| o.iter().any(|x| x == name)).unwrap_or(true)
    };

    if run("probe_latency", &only) {
        scenario_probe_latency(&mut bench).await;
    }
    if run("chunk_scaling", &only) {
        scenario_chunk_scaling(&mut bench, &samples).await;
    }
    if run("concurrency_scaling", &only) {
        scenario_concurrency(&mut bench, &samples).await;
    }
    if run("scheduler_queue", &only) {
        scenario_scheduler_queue(&mut bench).await;
    }
    if run("rate_limiting", &only) {
        scenario_rate_limit(&mut bench).await;
    }
    if run("edge_cases", &only) {
        scenario_edge_cases(&mut bench).await;
    }
    if run("small_files", &only) {
        scenario_small_files(&mut bench).await;
    }
    if run("storage_bench", &only) {
        scenario_storage(&mut bench).await;
    }
    if run("storage_impact", &only) {
        scenario_storage_impact(&mut bench).await;
    }
    if run("fix_verification", &only) {
        scenario_fix_verification(&mut bench).await;
    }
    if run("hls", &only) {
        scenario_hls(&mut bench).await;
    }
    if run("sha256", &only) {
        scenario_sha256(&mut bench).await;
    }
    if run("soak", &only) {
        scenario_soak(&mut bench).await;
    }

    stop.store(1, Ordering::Relaxed);
    let _ = monitor.join();

    let results = serde_json::json!({
        "environment": env,
        "mode": if quick { "quick" } else { "full" },
        "finished_at": chrono::Utc::now().to_rfc3339(),
        "scenarios": bench.results,
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&results).unwrap()).expect("write results");
    println!("结果已写入 {}", out_path.display());

    // 清理 scratch
    std::fs::remove_dir_all(&scratch).ok();
}
