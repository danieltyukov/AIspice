import { ThemeToggle } from "./ThemeToggle";
import "./StatusBar.css";

interface Props {
  activeFile: string;
}

export function StatusBar({ activeFile }: Props) {
  return (
    <div className="status-bar">
      <div className="status-bar__section">
        <span>{activeFile || "No file selected"}</span>
      </div>
      <div className="status-bar__spacer" />
      <div className="status-bar__section">
        <ThemeToggle />
      </div>
    </div>
  );
}
