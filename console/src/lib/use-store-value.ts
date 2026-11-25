import { useMemo, useSyncExternalStore } from "react";

/** A value held in a tiny external store instead of `useState`: read `value`, change it with `set`. */
export function useStoreValue<T>(initial: T): { value: T; set: (next: T) => void } {
  const store = useMemo(() => {
    let current = initial;
    const listeners = new Set<() => void>();
    return {
      get: () => current,
      set: (next: T) => {
        if (Object.is(next, current)) return;
        current = next;
        listeners.forEach((l) => l());
      },
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
    };
    // the initial value is only the first one
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const value = useSyncExternalStore(store.subscribe, store.get, store.get);
  return { value, set: store.set };
}
