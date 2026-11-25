import { useMemo } from "react";
import { useMappedSource, type ListSource } from "./listbox";
import { useLabelsSource, useTagsSource, useTasksSource } from "./sources";
import { useStoreValue } from "./use-store-value";

/** What a `TagInput` takes for server-backed options: the rows as strings, and the typed text to read again with. */
export interface TagOptions {
  source: ListSource<string>;
  onQuery: (q: string) => void;
}

const labelOf = (l: { label: string }) => l.label;
const tagOf = (t: { tag: string }) => t.tag;
const idOf = (t: { id: string }) => t.id;

/** A pot's labels as the options of a `TagInput`, narrowed by the server as you type. */
export function useLabelOptions(pot: string, enabled = true): TagOptions {
  const q = useStoreValue("");
  const source = useMappedSource(useLabelsSource(pot, { q: q.value, bucket: "all", enabled: enabled && !!pot }), labelOf);
  return { source, onQuery: q.set };
}

/** A pot's pantry tags as the options of a `TagInput`. */
export function useTagOptions(pot: string, enabled = true): TagOptions {
  const q = useStoreValue("");
  const source = useMappedSource(useTagsSource(pot, { q: q.value, enabled: enabled && !!pot }), tagOf);
  return { source, onQuery: q.set };
}

/** A pot's task ids (the ones not tossed) as the options of a `TagInput`: what a task can depend on. */
export function useTaskIdOptions(pot: string, enabled = true): TagOptions {
  const q = useStoreValue("");
  const tasks = useTasksSource(pot, { q: q.value, bucket: "all", enabled: enabled && !!pot });
  const live = useMemo(() => ({ ...tasks, items: tasks.items.filter((t) => t.status !== "cancelled") }), [tasks]);
  const source = useMappedSource(live, idOf);
  return { source, onQuery: q.set };
}
