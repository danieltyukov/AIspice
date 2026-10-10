import { useEffect, useState } from "react";
import { readJson, writeJson } from "../lib/storage";
import { Center } from "./Center";
import { ChatPanel } from "./chat/ChatPanel";
import { Sidebar } from "./Sidebar";
import { Splitter } from "./Splitter";
import { StatusBar } from "./StatusBar";
import "./ProjectView.css";

const SIDEBAR = { min: 180, max: 360 };
const CHAT = { min: 300, max: 620 };
const CENTER_MIN = 380;

function defaults(width: number) {
  return {
    sidebar: width < 1200 ? 208 : 236,
    chat: Math.round(Math.min(440, Math.max(320, width * 0.28))),
  };
}

/** Pane widths that fit the window, shrinking the chat first and the sidebar second. */
export function fitPanes(sidebar: number, chat: number, width: number): { sidebar: number; chat: number } {
  let s = Math.min(SIDEBAR.max, Math.max(SIDEBAR.min, sidebar));
  let c = Math.min(CHAT.max, Math.max(CHAT.min, chat));
  const room = width - CENTER_MIN - 2;
  if (s + c > room) c = Math.max(CHAT.min, room - s);
  if (s + c > room) s = Math.max(SIDEBAR.min, room - c);
  return { sidebar: s, chat: c };
}

function useWindowWidth(): number {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

export function ProjectView() {
  const width = useWindowWidth();
  const [sidebar, setSidebar] = useState(() => readJson<number | null>("sidebarW", null) ?? defaults(window.innerWidth).sidebar);
  const [chat, setChat] = useState(() => readJson<number | null>("chatW", null) ?? defaults(window.innerWidth).chat);
  const fitted = fitPanes(sidebar, chat, width);

  const changeSidebar = (v: number) => {
    setSidebar(v);
    writeJson("sidebarW", v);
  };
  const changeChat = (v: number) => {
    setChat(v);
    writeJson("chatW", v);
  };

  return (
    <div className="project">
      <div className="project-panes">
        <Sidebar width={fitted.sidebar} />
        <Splitter
          label="Resize circuit list"
          orientation="vertical"
          value={fitted.sidebar}
          min={SIDEBAR.min}
          max={Math.min(SIDEBAR.max, width - fitted.chat - CENTER_MIN)}
          onChange={changeSidebar}
          onReset={() => changeSidebar(defaults(width).sidebar)}
        />
        <Center />
        <Splitter
          label="Resize chat"
          orientation="vertical"
          invert
          value={fitted.chat}
          min={CHAT.min}
          max={Math.min(CHAT.max, width - fitted.sidebar - CENTER_MIN)}
          onChange={changeChat}
          onReset={() => changeChat(defaults(width).chat)}
        />
        <ChatPanel width={fitted.chat} />
      </div>
      <StatusBar />
    </div>
  );
}
