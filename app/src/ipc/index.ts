import type { Backend } from "./types";

/*
 * Which backend: the Tauri shell puts __TAURI_INTERNALS__ on the window before
 * the bundle runs, so its absence means a browser, Playwright or vitest, and
 * the in-memory mock takes over. Both are loaded lazily, which keeps the
 * Tauri modules out of the browser path and the mock out of the desktop path.
 */

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function loadBackend(): Promise<Backend> {
  if (inTauri()) {
    const { createTauriBackend } = await import("./api");
    return createTauriBackend();
  }
  const { createMockBackend } = await import("./mock");
  return createMockBackend();
}

export type * from "./types";
