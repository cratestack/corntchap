import { useMemo } from "react";
import type { ProcArgs } from "./data";
import { useProcInfinite, type PagedItem, type PagedProc } from "./paging";
import type { ListSource } from "./listbox";
import { useDebounced } from "./use-debounced";

/** How long the search text rests before it becomes the query. */
export const QUERY_DEBOUNCE_MS = 250;

export interface ListQueryOptions {
  /** The search text (typed live): sent as the procedure's `q` once it has rested for {@link QUERY_DEBOUNCE_MS}. */
  q?: string;
  /** The next-page cursor of an answer; default `nextCursor`, else `next_cursor` (a TaskPage). */
  getCursor?: (page: never) => string | null | undefined;
  /** Rows per request (the server's default 25 when left out). */
  pageSize?: number;
  enabled?: boolean;
}

const defaultCursor = (page: { nextCursor?: string | null; next_cursor?: string | null }) => page.nextCursor ?? page.next_cursor;

/**
 * A `*Page` procedure as a {@link ListSource} for the pickers: pages as the list is scrolled, `q` (debounced)
 * narrows it on the server, and it keeps every page loaded (`maxPages` unbounded: the list is virtualized).
 * `args` are the procedure's own arguments without `page`; send `q` through the option, not in `args`.
 */
export function useListQuery<N extends PagedProc>(name: N, args: Omit<ProcArgs<N>, "page">, opts: ListQueryOptions = {}): ListSource<PagedItem<N>> {
  const q = useDebounced(opts.q ?? "", QUERY_DEBOUNCE_MS).trim();
  const withQ = useMemo(() => (q ? { ...args, q } : args) as Omit<ProcArgs<N>, "page">, [args, q]);
  const r = useProcInfinite(name, withQ, {
    getCursor: (opts.getCursor ?? defaultCursor) as never,
    maxPages: Infinity,
    ...(opts.pageSize ? { pageSize: opts.pageSize } : {}),
    ...(opts.enabled === undefined ? {} : { enabled: opts.enabled }),
  });
  const { items, hasNextPage, fetchNextPage, isLoading, isFetchingNextPage, error, capped, total } = r;
  return useMemo<ListSource<PagedItem<N>>>(
    () => ({ items: items as PagedItem<N>[], hasNextPage, fetchNextPage: () => void fetchNextPage(), isLoading, isFetchingNextPage, error, capped, total }),
    [items, hasNextPage, fetchNextPage, isLoading, isFetchingNextPage, error, capped, total],
  );
}
