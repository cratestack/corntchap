import { useMemo, useSyncExternalStore } from "react";

/**
 * The text of values already picked, kept in a tiny external store (not state): a trigger or chip shows the text of
 * what was picked even when its row is no longer in the loaded rows (a server search narrowed the source, or the value
 * came back from the URL). `remember` takes `[key, text]` pairs; `labels` is read at render.
 */
export function useLabelCache() {
  const store = useMemo(() => {
    let labels: Readonly<Record<string, string>> = {};
    const listeners = new Set<() => void>();
    return {
      get: () => labels,
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
      remember: (entries: readonly (readonly [string, string])[]) => {
        let next: Record<string, string> | undefined;
        for (const [k, v] of entries) if (labels[k] !== v) (next ??= { ...labels })[k] = v;
        if (!next) return;
        labels = next;
        listeners.forEach((l) => l());
      },
    };
  }, []);
  const labels = useSyncExternalStore(store.subscribe, store.get, store.get);
  return { labels, remember: store.remember };
}
