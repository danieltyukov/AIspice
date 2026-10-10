import { useStore, useStoreApi } from "../store/context";
import "./Toasts.css";

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const store = useStoreApi();
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast" data-tone={t.tone} role={t.tone === "error" ? "alert" : "status"}>
          <p className="toast-text">{t.text}</p>
          <button type="button" className="toast-close" onClick={() => store.dismissToast(t.id)} aria-label="Dismiss">
            Dismiss
          </button>
        </div>
      ))}
    </div>
  );
}
