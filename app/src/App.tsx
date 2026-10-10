import { useEffect } from "react";
import { hasMod, inTextField } from "./lib/keys";
import { useStore, useStoreApi } from "./store/context";
import { TABS } from "./store/store";
import { ProjectView } from "./components/ProjectView";
import { SettingsDialog } from "./components/SettingsDialog";
import { ShortcutsSheet } from "./components/ShortcutsSheet";
import { Toasts } from "./components/Toasts";
import { Welcome } from "./components/Welcome";

/** Window-wide shortcuts. Field-sensitive ones stand aside while typing. */
function useShortcuts() {
  const store = useStoreApi();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = store.get();
      const modal = s.settingsOpen || s.shortcutsOpen;
      const typing = inTextField(e.target);
      const key = e.key.toLowerCase();

      if (hasMod(e) && !e.altKey) {
        if (key === "o") {
          e.preventDefault();
          if (!modal) void store.openFolder();
        } else if (key === ",") {
          e.preventDefault();
          store.openSettings();
        } else if (key === "r" && !e.shiftKey) {
          e.preventDefault();
          if (!modal && s.project) void store.simulate();
        } else if (key === "enter") {
          if (!modal && s.project) {
            e.preventDefault();
            window.dispatchEvent(new Event("aispice:send"));
          }
        } else if (key === "z" && !typing && !modal) {
          e.preventDefault();
          if (e.shiftKey) void store.redo();
          else void store.undo();
        } else if (key === "y" && !typing && !modal) {
          e.preventDefault();
          void store.redo();
        } else if (key === "\\" && !modal && s.project) {
          e.preventDefault();
          store.toggleSplit();
        } else if (/^[1-6]$/.test(e.key) && !modal && s.project) {
          e.preventDefault();
          store.setTab(TABS[Number(e.key) - 1][0]);
        }
        return;
      }

      if (e.key === "?" && !typing && !modal) {
        e.preventDefault();
        store.setShortcutsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [store]);
}

export function App() {
  const store = useStoreApi();
  const booted = useStore((s) => s.booted);
  const project = useStore((s) => s.project);
  const settingsOpen = useStore((s) => s.settingsOpen);
  const shortcutsOpen = useStore((s) => s.shortcutsOpen);
  useShortcuts();

  useEffect(() => {
    void store.boot();
    return () => store.dispose();
  }, [store]);

  if (!booted) return <div className="startup">Starting...</div>;

  return (
    <>
      {project ? <ProjectView /> : <Welcome />}
      {settingsOpen ? <SettingsDialog /> : null}
      {shortcutsOpen ? <ShortcutsSheet /> : null}
      <Toasts />
    </>
  );
}
