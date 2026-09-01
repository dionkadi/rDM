// Formatting utilities — used across all components for consistent display

export function fmtBytes(n: number | null | undefined, decimals = 1): string {
  if (n == null) return "—";
  if (n === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(k)), sizes.length - 1);
  return `${(n / Math.pow(k, i)).toFixed(i === 0 ? 0 : decimals)} ${sizes[i]}`;
}

export function fmtRate(bps: number | null | undefined, decimals = 1): string {
  if (bps == null || bps < 0) return "—";
  if (bps === 0) return "0 B/s";
  const k = 1024;
  const sizes = ["B/s", "KB/s", "MB/s", "GB/s"];
  const i = Math.min(Math.floor(Math.log(bps) / Math.log(k)), sizes.length - 1);
  return `${(bps / Math.pow(k, i)).toFixed(i === 0 ? 0 : decimals)} ${sizes[i]}`;
}

export function fmtPercent(downloaded: number, total: number | null | undefined): string {
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

export function fmtTime(hours: number, minutes: number): string {
  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`;
}

export function fmtRelativeTime(date: string | Date | null | undefined): string {
  if (!date) return "—";
  const d = typeof date === "string" ? new Date(date) : date;
  const diff = Date.now() - d.getTime();
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

export function fmtDate(date: string | Date | null | undefined): string {
  if (!date) return "—";
  const d = typeof date === "string" ? new Date(date) : date;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - 1) + "…";
}

export function pad2(n: number): string {
  return n.toString().padStart(2, "0");
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

export function debounce<T extends (...args: any[]) => unknown>(
  fn: T,
  ms: number,
): (...args: Parameters<T>) => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return (...args) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

export function throttle<T extends (...args: any[]) => unknown>(
  fn: T,
  ms: number,
): (...args: Parameters<T>) => void {
  let last = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  return (...args) => {
    const now = Date.now();
    if (now - last >= ms) {
      last = now;
      fn(...args);
    } else {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        last = Date.now();
        fn(...args);
      }, ms - (now - last));
    }
  };
}