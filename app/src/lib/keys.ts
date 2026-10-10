/* Keyboard helpers shared by the global shortcuts and the help sheet. */

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** The modifier label: "Cmd" on macOS, "Ctrl" elsewhere. */
export const MOD = isMac ? "Cmd" : "Ctrl";

export function hasMod(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/** True when typing would go into a field, so single-key and undo shortcuts must stand aside. */
export function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "submit", "range", "color"].includes(type);
  }
  return false;
}

export interface Shortcut {
  keys: string[];
  label: string;
}

export const SHORTCUTS: ReadonlyArray<{ group: string; items: Shortcut[] }> = [
  {
    group: "Project",
    items: [
      { keys: [MOD, "O"], label: "Open a folder" },
      { keys: [MOD, ","], label: "Settings" },
      { keys: ["?"], label: "This list" },
    ],
  },
  {
    group: "Circuit",
    items: [
      { keys: [MOD, "R"], label: "Simulate" },
      { keys: [MOD, "Z"], label: "Undo" },
      { keys: [MOD, "Shift", "Z"], label: "Redo" },
      { keys: [MOD, "1-6"], label: "Switch tab" },
      { keys: [MOD, "\\"], label: "Split view" },
    ],
  },
  {
    group: "Schematic",
    items: [
      { keys: ["Arrows"], label: "Pan" },
      { keys: ["+", "-"], label: "Zoom" },
      { keys: ["F"], label: "Fit to view" },
      { keys: ["Esc"], label: "Close popover, clear highlights" },
    ],
  },
  {
    group: "Chat",
    items: [
      { keys: ["Enter"], label: "Send" },
      { keys: ["Shift", "Enter"], label: "New line" },
      { keys: [MOD, "Enter"], label: "Send from anywhere" },
    ],
  },
];
