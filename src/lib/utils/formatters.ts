// Formatting utilities — used across all components for consistent display

/**
 * Format a byte count as a human-readable string. The unit array
 * extends to **YB** (yottabyte) so values up to ~2^80 render as
 * "1024.0 YB" instead of the old "16777216.0 TB" bug (where
 * `u64::MAX` rounded through a double and then divided by
 * `1024^4` produced a meaningless number). Anything above 1024 YB
 * renders as ">1024 YB" rather than overflowing into scientific
 * notation or `Infinity`.
 */
export function fmtBytes(n: number | null | undefined, decimals = 1): string {
  if (n == null) return "—";
  if (n === 0) return "0 B";
  if (!Number.isFinite(n) || n < 0) return "—";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(k)), sizes.length - 1);
  if (i === sizes.length - 1 && n >= k ** i) {
    // Top of the ladder. Don't divide further; show the cap with
    // a `+` so the user knows the value is at least this big.
    return `≥1024 ${sizes[i]}`;
  }
  return `${(n / k ** i).toFixed(i === 0 ? 0 : decimals)} ${sizes[i]}`;
}

export function fmtRate(bps: number | null | undefined, decimals = 1): string {
  if (bps == null || bps < 0) return "—";
  if (bps === 0) return "0 B/s";
  const k = 1024;
  const sizes = ["B/s", "KB/s", "MB/s", "GB/s"];
  const i = Math.min(Math.floor(Math.log(bps) / Math.log(k)), sizes.length - 1);
  return `${(bps / k ** i).toFixed(i === 0 ? 0 : decimals)} ${sizes[i]}`;
}

export function fmtPercent(
  downloaded: number,
  total: number | null | undefined,
): string {
  if (!total || total === 0) return "?";
  return `${((downloaded / total) * 100).toFixed(1)}%`;
}

export function fmtDuration(sec: number | null | undefined): string {
  if (sec == null || sec < 0) return "—";
  if (sec === 0) return "0s";
  if (sec < 60) return `${Math.round(sec)}s`;
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  const mm = m % 60;
  if (h < 24) return `${h}h ${mm}m`;
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}

export function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - 1) + "…";
}

export function filenameFromUrl(u: string): string {
  try {
    const path = new URL(u).pathname;
    const last = path.split("/").pop() || "";
    if (last) return decodeURIComponent(last);
  } catch {
    /* not a URL */
  }
  const fallback = u.split("/").pop()?.split("?")[0] || "download";
  return fallback || "download";
}

export function extOf(name: string): string {
  return name.split(".").pop()?.toLowerCase() || "";
}

export function isUrl(s: string): boolean {
  return /^https?:\/\/\S+$/i.test(s.trim());
}
