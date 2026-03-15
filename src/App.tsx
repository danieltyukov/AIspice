import { useState, useCallback } from "react";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { Chat } from "./components/Chat";
import { Settings } from "./components/Settings";
import "./styles/theme.css";
import "./styles/global.css";

type View = "welcome" | "editor";

function App() {
  const [view, setView] = useState<View>("welcome");
  const [files, setFiles] = useState<string[]>([]);
  const [activeFile, setActiveFile] = useState<string | null>(null);
  const [folderName, setFolderName] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);

  const openFolder = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const { invoke } = await import("@tauri-apps/api/core");

      const dir = await open({ directory: true });
      if (!dir) return;

      const dirStr = typeof dir === "string" ? dir : String(dir);

      // Tell the backend about the working directory
      await invoke("set_working_directory", { dir: dirStr });

      // List all circuit files in the folder
      const foundFiles = await invoke<string[]>("list_circuit_files");

      const parts = dirStr.split(/[/\\]/);
      setFolderName(parts[parts.length - 1] || "project");
      setFiles(foundFiles);

      // Auto-select the first file
      if (foundFiles.length > 0) {
        setActiveFile(foundFiles[0]);
      }

      setView("editor");
    } catch {
      // Outside Tauri — switch to editor for testing
      setView("editor");
    }
  };

  const handleFileTabClick = useCallback((filename: string) => {
    setActiveFile(filename);
  }, []);

  if (view === "welcome") {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          gap: 24,
          fontFamily: "var(--font-sans)",
          color: "var(--text-primary)",
          background: "var(--bg-primary)",
        }}
      >
        <img
          src="/src/assets/icon.svg"
          alt="AIspice icon"
          style={{ width: 88, height: 88 }}
          onError={(e) => {
            (e.target as HTMLImageElement).style.display = "none";
          }}
        />
        <img
          src="/src/assets/logo.svg"
          alt="AIspice"
          style={{ height: 56 }}
          onError={(e) => {
            (e.target as HTMLImageElement).style.display = "none";
          }}
        />
        <p
          style={{
            color: "var(--text-secondary)",
            fontSize: 16,
            margin: 0,
          }}
        >
          AI-powered LTspice circuit assistant
        </p>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 12,
            marginTop: 16,
            width: 300,
            alignItems: "stretch",
          }}
        >
          <button
            onClick={openFolder}
            style={{
              padding: "10px 14px",
              fontSize: 14,
              fontWeight: 600,
              color: "var(--text-inverse)",
              background: "var(--color-primary)",
              borderRadius: 6,
              cursor: "pointer",
              transition: "background 0.15s",
            }}
            onMouseEnter={(e) =>
              (e.currentTarget.style.background = "var(--color-primary-light)")
            }
            onMouseLeave={(e) =>
              (e.currentTarget.style.background = "var(--color-primary)")
            }
          >
            Open Project Folder
          </button>
          <button
            onClick={() => setSettingsOpen(true)}
            style={{
              padding: "8px 14px",
              fontSize: 13,
              fontWeight: 500,
              color: "var(--text-secondary)",
              background: "transparent",
              border: "1px solid var(--border-light)",
              borderRadius: 6,
              cursor: "pointer",
              transition: "background 0.15s",
            }}
            onMouseEnter={(e) =>
              (e.currentTarget.style.background = "var(--bg-secondary)")
            }
            onMouseLeave={(e) =>
              (e.currentTarget.style.background = "transparent")
            }
          >
            Settings (API Keys)
          </button>
        </div>

        <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      </div>
    );
  }

  // Editor view
  const RAINBOW = [
    "var(--rainbow-1)",
    "var(--rainbow-2)",
    "var(--rainbow-3)",
    "var(--rainbow-4)",
    "var(--rainbow-5)",
    "var(--rainbow-6)",
  ];

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        overflow: "hidden",
      }}
    >
      <TitleBar title={`AIspice — ${folderName}`} />

      {/* File tabs bar */}
      {files.length > 0 && (
        <div
          style={{
            display: "flex",
            gap: 0,
            borderBottom: "1px solid var(--border-light)",
            background: "var(--bg-secondary)",
            overflowX: "auto",
            flexShrink: 0,
          }}
        >
          {files.map((f, i) => {
            const isActive = f === activeFile;
            const color = RAINBOW[i % RAINBOW.length];
            const shortName = f.split("/").pop() || f;
            return (
              <button
                key={f}
                onClick={() => handleFileTabClick(f)}
                title={f}
                style={{
                  padding: "8px 16px",
                  fontSize: 12,
                  fontFamily: "var(--font-mono)",
                  borderBottom: `2px solid ${isActive ? color : "transparent"}`,
                  color: isActive
                    ? "var(--text-primary)"
                    : "var(--text-secondary)",
                  background: isActive ? "var(--bg-primary)" : "transparent",
                  whiteSpace: "nowrap",
                  cursor: "pointer",
                  transition: "all 0.1s",
                }}
              >
                {shortName}
              </button>
            );
          })}
        </div>
      )}

      {/* Chat takes full remaining space */}
      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        <Chat activeFile={activeFile} />
      </div>

      <StatusBar activeFile={activeFile ?? ""} />
    </div>
  );
}

export default App;
