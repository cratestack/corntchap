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
  /** The next-page cursor of an answer; default `nextCursor`. */
  getCursor?: (page: never) => string | null | undefined;
  /** Rows per request (the server's default 25 when left out). */
  pageSize?: number;
  enabled?: boolean;
}

const defaultCursor = (page: { nextCursor?: string | null }) => page.nextCursor;

/**
 * A `*Page` procedure as a {@link ListSource} for the pickers: pages as the list is scrolled, `q` (debounced)
 * narrows it on the server (the previous rows stay, `pending`, until its answer comes), and it keeps every page loaded (`maxPages` unbounded: the list is virtualized).
 * `args` are the procedure's own arguments without `page`; send `q` through the option, not in `args`.
 */
export function useListQuery<N extends PagedProc>(name: N, args: Omit<ProcArgs<N>, "page">, opts: ListQueryOptions = {}): ListSource<PagedItem<N>> {
  const raw = (opts.q ?? "").trim();
  const q = useDebounced(opts.q ?? "", QUERY_DEBOUNCE_MS).trim();
  const withQ = useMemo(() => (q ? { ...args, q } : args) as Omit<ProcArgs<N>, "page">, [args, q]);
  const r = useProcInfinite(name, withQ, {
    getCursor: (opts.getCursor ?? defaultCursor) as never,
    maxPages: Infinity,
    keepPrevious: true,
    ...(opts.pageSize ? { pageSize: opts.pageSize } : {}),
    ...(opts.enabled === undefined ? {} : { enabled: opts.enabled }),
  });
  const { items, hasNextPage, fetchNextPage, isLoading, isFetchingNextPage, error, capped, total, isPlaceholderData } = r;
  // the rows are still the previous query's: the text has not rested yet, or the new query's first page is loading
  const pending = raw !== q || isPlaceholderData;
  return useMemo<ListSource<PagedItem<N>>>(
    () => ({
      items: items as PagedItem<N>[],
      hasNextPage,
      // another page of the previous query is no use to the new one
      fetchNextPage: () => {
        if (!pending) void fetchNextPage();
      },
      isLoading,
      isFetchingNextPage,
      error,
      capped,
      total,
      pending,
    }),
    [items, hasNextPage, fetchNextPage, isLoading, isFetchingNextPage, error, capped, total, pending],
  );
}
