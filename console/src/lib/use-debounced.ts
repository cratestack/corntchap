import { useEffect, useMemo, useSyncExternalStore } from "react";

/** `value`, but only after it has stayed the same for `ms`: a search box's text becoming the query. A tiny external store, not state. */
export function useDebounced<T>(value: T, ms: number): T {
  const store = useMemo(() => {
    let current = value;
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const t = setTimeout(() => store.set(value), ms);
    return () => clearTimeout(t);
  }, [value, ms, store]);
  return useSyncExternalStore(store.subscribe, store.get, store.get);
}
