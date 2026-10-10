import type { ChangeInfo } from "../hooks/useChat";
import "./FileChange.css";

interface FileChangeProps {
  change: ChangeInfo;
}

export function FileChange({ change }: FileChangeProps) {
  return (
    <span className="file-change">
      {change.component && (
        <span className="file-change__component">{change.component}</span>
      )}
      <span className="file-change__description">{change.description}</span>
      <span className="file-change__filename">{change.filename}</span>
    </span>
  );
}
