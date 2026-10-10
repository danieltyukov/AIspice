/*
 * localStorage for per-window conveniences: pane sizes, the split view, the
 * theme for the first paint. Storage can be missing or throw (private mode,
 * a locked-down WebView), so every access is guarded and the app works
 * without it.
 */

const PREFIX = "aispice.";

export function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // Not persisted this time; nothing else depends on it.
  }
}
