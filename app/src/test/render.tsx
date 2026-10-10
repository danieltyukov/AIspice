import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { createMockBackend } from "../ipc/mock";
import type { Backend } from "../ipc/types";
import { StoreProvider } from "../store/context";
import { Store } from "../store/store";

/** Renders a component inside a store backed by the fast mock. */
export async function renderWithStore(ui: ReactElement, options: { backend?: Backend; project?: boolean } = {}) {
  const store = new Store(options.backend ?? createMockBackend({ fast: true }));
  await store.boot();
  if (options.project) await store.openProject("/home/you/circuits/filters");
  const result = render(<StoreProvider store={store}>{ui}</StoreProvider>);
  return { store, ...result };
}
