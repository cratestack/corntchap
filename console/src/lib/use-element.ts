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

/**
 * Whether an element is at least `rem` wide, kept current by a ResizeObserver and held in a tiny external store
 * instead of `useState`: attach `ref` to the node and read `fits` (null until it is measured, which happens before
 * the first paint). Re-renders only when the answer flips, not on every resize. For a layout that needs different
 * components (not just different CSS) depending on the room its container has.
 */
export function useAtLeastWide<T extends HTMLElement = HTMLElement>(rem: number): { fits: boolean | null; ref: (node: T | null) => void } {
  const store = useMemo(() => {
    let node: T | null = null;
    let observer: ResizeObserver | null = null;
    let current: boolean | null = null;
    const listeners = new Set<() => void>();
    const measure = () => {
      const px = rem * (parseFloat(getComputedStyle(document.documentElement).fontSize) || 16);
      const next = node ? node.getBoundingClientRect().width >= px - 0.5 : null;
      if (next === current) return;
      current = next;
      listeners.forEach((l) => l());
    };
    return {
      get: () => current,
      ref: (n: T | null) => {
        if (n === node) return;
        observer?.disconnect();
        observer = null;
        node = n;
        if (n) {
          observer = new ResizeObserver(measure);
          observer.observe(n);
        }
        measure();
      },
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
    };
  }, [rem]);
  const fits = useSyncExternalStore(store.subscribe, store.get, () => null);
  return { fits, ref: store.ref };
}
