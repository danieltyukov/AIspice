import { SHORTCUTS } from "../lib/keys";
import { useStoreApi } from "../store/context";
import { Dialog } from "./Dialog";
import "./ShortcutsSheet.css";

export function ShortcutsSheet() {
  const store = useStoreApi();
  return (
    <Dialog
      title="Keyboard shortcuts"
      onClose={() => store.setShortcutsOpen(false)}
      wide
      footer={
        <button type="button" className="button" onClick={() => store.setShortcutsOpen(false)}>
          Close
        </button>
      }
    >
      <div className="shortcuts">
        {SHORTCUTS.map((g) => (
          <section key={g.group} className="shortcut-group" aria-label={g.group}>
            <h3 className="section-title">{g.group}</h3>
            <dl className="shortcut-list">
              {g.items.map((s) => (
                <div key={s.label} className="shortcut">
                  <dt>{s.label}</dt>
                  <dd>
                    {s.keys.map((k) => (
                      <kbd key={k}>{k}</kbd>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}
