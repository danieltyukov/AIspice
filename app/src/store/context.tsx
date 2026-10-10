import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";
import type { AppState, Store } from "./store";

const StoreContext = createContext<Store | null>(null);

export function StoreProvider({ store, children }: { store: Store; children: ReactNode }) {
  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>;
}

export function useStoreApi(): Store {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useStoreApi needs a StoreProvider above it");
  return store;
}

/** Subscribes to one slice. The selector must return a value already in state, not a new object. */
export function useStore<T>(selector: (s: AppState) => T): T {
  const store = useStoreApi();
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.get()),
    () => selector(store.get()),
  );
}
