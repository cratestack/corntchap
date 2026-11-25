import { useCallback, useMemo, useSyncExternalStore } from "react";

/**
 * A DOM element held in a tiny external store instead of `useState`: attach `ref` to a node and read `element`
 * (null until it mounts) to hand it to something that needs the node itself during render, such as a virtualized
 * list's custom scroll parent.
 */
export function useElement<T extends HTMLElement = HTMLElement>(): { element: T | null; ref: (node: T | null) => void } {
  const store = useMemo(() => {
    let current: T | null = null;
    const listeners = new Set<() => void>();
    return {
      get: () => current,
      set: (node: T | null) => {
        if (node === current) return;
        current = node;
        listeners.forEach((l) => l());
      },
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
    };
  }, []);
  const element = useSyncExternalStore(store.subscribe, store.get, () => null);
  const ref = useCallback((node: T | null) => store.set(node), [store]);
  return { element, ref };
}
