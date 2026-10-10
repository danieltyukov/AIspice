import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { loadBackend } from "./ipc";
import { applyTheme, readTheme } from "./lib/theme";
import { StoreProvider } from "./store/context";
import { Store } from "./store/store";
import "./tokens.css";
import "./base.css";

// Before the first paint, so a dark theme does not flash light.
applyTheme(readTheme());

const container = document.getElementById("root");
if (!container) throw new Error("root element not found");
const root = createRoot(container);

loadBackend().then(
  (backend) => {
    const store = new Store(backend);
    root.render(
      <StrictMode>
        <StoreProvider store={store}>
          <App />
        </StoreProvider>
      </StrictMode>,
    );
  },
  (error: unknown) => {
    console.error("backend failed to load", error);
    root.render(
      <p role="alert" className="startup">
        aispice could not reach its backend. Close the window and open it again.
      </p>,
    );
  },
);
