import { useEffect, useState } from "react";
import { readJson, writeJson } from "./storage";

/*
 * The theme is a data-theme attribute on the root element; "system" is its
 * absence, which leaves the media query in tokens.css in charge. The backend
 * settings hold the choice; localStorage keeps a copy so the first paint is
 * already right before settings arrive.
 */

export type Theme = "system" | "light" | "dark";

const KEY = "theme";

export function readTheme(): Theme {
  const t = readJson<string>(KEY, "system");
  return t === "light" || t === "dark" ? t : "system";
}

export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
  writeJson(KEY, theme);
}

/** True when the dark palette is in effect, whichever way it was chosen. */
export function isDark(): boolean {
  const attr = document.documentElement.getAttribute("data-theme");
  if (attr === "dark") return true;
  if (attr === "light") return false;
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * A counter that changes whenever the effective palette may have changed, for
 * canvas drawings (plots) that read tokens once and must redraw.
 */
export function usePaletteVersion(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const bump = () => setVersion((v) => v + 1);
    const observer = new MutationObserver(bump);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const media = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null;
    media?.addEventListener("change", bump);
    return () => {
      observer.disconnect();
      media?.removeEventListener("change", bump);
    };
  }, []);
  return version;
}

/** Reads a token's current value from the root element. */
export function token(name: string, fallback = "#888"): string {
  if (typeof getComputedStyle !== "function") return fallback;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}
