import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import "./Dialog.css";

const FOCUSABLE =
  'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])';

export interface DialogProps {
  title: string;
  description?: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  /** Element to focus first; defaults to the first focusable control. */
  initialFocus?: string;
}

/*
 * The one modal shape. Focus moves in on open and back to the opener on
 * close; Tab and Shift+Tab stay inside; Escape closes. Portalled to the body
 * so a pane's overflow never clips it.
 */
export function Dialog({ title, description, onClose, children, footer, wide, initialFocus }: DialogProps) {
  const surface = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descId = useId();

  useEffect(() => {
    const returnTo = document.activeElement;
    const root = surface.current;
    const first = (initialFocus ? root?.querySelector<HTMLElement>(initialFocus) : null) ?? root?.querySelector<HTMLElement>(FOCUSABLE);
    first?.focus();
    return () => {
      if (returnTo instanceof HTMLElement) returnTo.focus();
    };
    // Runs once per open; initialFocus is read at mount only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const items = [...(surface.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div
      className="scrim"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={surface}
        className="dialog"
        data-wide={wide ? "" : undefined}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descId : undefined}
        onKeyDown={onKeyDown}
      >
        <header className="dialog-head">
          <h2 id={titleId} className="dialog-title">
            {title}
          </h2>
          {description ? (
            <p id={descId} className="dialog-text">
              {description}
            </p>
          ) : null}
        </header>
        <div className="dialog-body">{children}</div>
        {footer ? <footer className="dialog-actions">{footer}</footer> : null}
      </div>
    </div>,
    document.body,
  );
}
