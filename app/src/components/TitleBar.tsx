import { useEffect, useState } from "react";
import "./TitleBar.css";

interface TitleBarProps {
  title: string;
}

export function TitleBar({ title }: TitleBarProps) {
  const [isMacOS, setIsMacOS] = useState(false);

  useEffect(() => {
    setIsMacOS(navigator.platform.toUpperCase().includes("MAC"));
  }, []);

  if (!isMacOS) {
    return null;
  }

  const handleClose = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().close();
    } catch {
      // Outside Tauri
    }
  };

  const handleMinimize = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().minimize();
    } catch {
      // Outside Tauri
    }
  };

  const handleMaximize = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (await win.isMaximized()) {
        await win.unmaximize();
      } else {
        await win.maximize();
      }
    } catch {
      // Outside Tauri
    }
  };

  return (
    <div className="title-bar" data-tauri-drag-region>
      <div className="title-bar__traffic-lights">
        <button
          className="title-bar__btn title-bar__btn--close"
          onClick={handleClose}
          aria-label="Close"
        />
        <button
          className="title-bar__btn title-bar__btn--minimize"
          onClick={handleMinimize}
          aria-label="Minimize"
        />
        <button
          className="title-bar__btn title-bar__btn--maximize"
          onClick={handleMaximize}
          aria-label="Maximize"
        />
      </div>
      <div className="title-bar__title">{title}</div>
    </div>
  );
}
