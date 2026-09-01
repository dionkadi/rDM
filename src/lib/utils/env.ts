// Environment detection helpers
export const browser = typeof window !== "undefined" && typeof document !== "undefined";
export const isMac = browser && /Mac|iPhone|iPad/i.test(navigator.platform);