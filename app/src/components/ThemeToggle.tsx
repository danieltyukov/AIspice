import { useState } from "react";

export function ThemeToggle() {
  const [dark, setDark] = useState(false);

  const toggle = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute(
      "data-theme",
      next ? "dark" : "light",
    );
  };

  return (
    <button
      onClick={toggle}
      className="theme-toggle"
      title="Toggle theme"
      style={{
        border: "none",
        background: "none",
        fontSize: 14,
        cursor: "pointer",
        padding: "0 4px",
        color: "var(--text-secondary)",
        lineHeight: 1,
      }}
    >
      {dark ? "\u2600" : "\u263E"}
    </button>
  );
}
