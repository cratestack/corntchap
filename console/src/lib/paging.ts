import { useMemo } from "react";
import { useInfiniteQuery, type InfiniteData, type Query, type QueryClient } from "@tanstack/react-query";
import { dataProvider } from "./api";
import type { ProcArgs, ProcName, ProcOut } from "./data";

/** Most pages one list keeps loaded; past it the reader is told, not given a slower page. */
export const MAX_PAGES = 20;

/** A `*Page` procedure's answer: the rows, the next cursor (either spelling), and sometimes the total. */
interface PageShape {
  items: unknown[];
  total?: number | null | undefined;
}

/** The procedures whose answer is a page. */
export type PagedProc = { [N in ProcName]: ProcOut<N> extends PageShape ? N : never }[ProcName];
export type PagedItem<N extends PagedProc> = ProcOut<N> extends { items: (infer I)[] } ? I : never;

/** What `invalidateData` needs to refresh a paging query without refetching every page. */
export interface PagingMeta {
  /** The procedure and its arguments, when the list is a `*Page` procedure. */
  name?: string;
  args?: unknown;
  pageSize: number;
  /** Most rows one request may ask for (the server clamps to it). */
  maxLimit: number;
  /** Read one page: `limit` rows after `cursor` (null: the first page). */
  load: (limit: number, cursor: string | null) => Promise<Paged>;
  getCursor: (page: never) => string | null | undefined;
  itemKey: (item: never) => string;
}

export interface PagedOptions<Page extends PageShape> {
  /** The cursor of the next page in an answer: `nextCursor` of a CursorPage, `next_cursor` of a TaskPage. */
  getCursor: (page: Page) => string | null | undefined;
  /** Rows per page; the server's default when left out. */
  pageSize?: number;
  /** The server's most rows per request (default 200). */
  maxLimit?: number;
  /** A row's identity, to drop the rows a refresh shows twice (default: its `id`, else its `key`). */
  itemKey?: (item: Page["items"][number]) => string;
  enabled?: boolean;
  /**
   * Most pages the list keeps loadable (default {@link MAX_PAGES}; `Infinity` for none). A virtualized list bounds
   * the DOM, so it can go on for as long as the reader scrolls.
   */
  maxPages?: number;
}

export type ProcInfiniteOptions<N extends PagedProc> = PagedOptions<ProcOut<N>>;

const defaultKey = (item: unknown): string => {
  const r = item as { id?: string; key?: string };
  return r.id ?? r.key ?? JSON.stringify(item);
};

type Paged = PageShape & Record<string, unknown>;

function call(name: string, args: unknown): Promise<Paged> {
  return (dataProvider.custom as NonNullable<typeof dataProvider.custom>)({
    url: "",
    method: "get",
    meta: { procedure: name },
    payload: args as Record<string, unknown>,
  }).then((res) => (res as { data: Paged }).data);
}

/**
 * Any cursor-paged read as an infinite list. `load` reads one page; the query lives under `["data", "inf"]`, so the
 * live poller refreshes it through `invalidateData` (which keeps the pages the reader has loaded instead of
 * refetching all of them). No `maxPages`: dropping the head would shift what the reader is looking at.
 */
export function usePagedQuery<Page extends PageShape>(
  key: readonly unknown[],
  load: (limit: number, cursor: string | null) => Promise<Page>,
  opts: PagedOptions<Page>,
  extra: { name?: string; args?: unknown } = {},
) {
  const pageSize = opts.pageSize ?? 25;
  const maxLimit = opts.maxLimit ?? 200;
  const getCursor = opts.getCursor;
  const maxPages = opts.maxPages ?? MAX_PAGES;
  const meta: { paging: PagingMeta } = {
    paging: {
      ...(extra.name ? { name: extra.name } : {}),
      ...(extra.args !== undefined ? { args: extra.args } : {}),
      pageSize,
      maxLimit,
      load: load as PagingMeta["load"],
      getCursor: getCursor as PagingMeta["getCursor"],
      itemKey: (opts.itemKey ?? defaultKey) as PagingMeta["itemKey"],
    },
  };
  const q = useInfiniteQuery({
    queryKey: ["data", "inf", ...key],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) => load(pageSize, pageParam),
    getNextPageParam: (last, all) => (all.length >= maxPages ? undefined : (getCursor(last) ?? undefined)),
    staleTime: 30_000,
    refetchOnWindowFocus: false,
    enabled: opts.enabled ?? true,
    meta,
  });
  const pages = q.data?.pages;
  const items = useMemo(() => (pages ?? []).flatMap((p) => p.items) as Page["items"], [pages]);
  const total = pages?.[0]?.total ?? null;
  return {
    items,
    total,
    isLoading: q.isLoading,
    error: q.error as unknown,
    fetchNextPage: q.fetchNextPage,
    hasNextPage: q.hasNextPage,
    isFetchingNextPage: q.isFetchingNextPage,
    /** The most pages one list holds are loaded. */
    capped: (pages?.length ?? 0) >= maxPages,
  };
}

/** A `*Page` procedure as an infinite list: `page: { limit, cursor }` is added to `args`. */
export function useProcInfinite<N extends PagedProc>(name: N, args: Omit<ProcArgs<N>, "page">, opts: ProcInfiniteOptions<N>) {
  return usePagedQuery<ProcOut<N>>(
    [name, args],
    (limit, cursor) => call(name, { ...args, page: { limit, cursor } }) as Promise<ProcOut<N>>,
    opts,
    { name, args },
  );
}

const refreshing = new Map<string, boolean>();

/** Refresh the head of a paging query: the first page re-read with room for what a page more would hold. */
async function refreshHead(qc: QueryClient, q: Query) {
  const hash = q.queryHash;
  if (refreshing.has(hash)) {
    refreshing.set(hash, true); // something changed while it ran: once more after
    return;
  }
  refreshing.set(hash, false);
  try {
    const paging = (q.meta as { paging?: PagingMeta } | undefined)?.paging;
    const data = q.state.data as InfiniteData<Paged, string | null> | undefined;
    const head = data?.pages[0];
    if (!paging || !data || !head) return;
    // One page loaded: read as many as it held (at least a page's worth). Several: read the first page's worth plus
    // one more, so the later pages the reader loaded can follow on from it without a gap.
    const several = data.pages.length > 1;
    const limit = Math.min(several ? head.items.length + paging.pageSize : Math.max(head.items.length, paging.pageSize), paging.maxLimit);
    const fresh = await paging.load(limit, null);
    const now = qc.getQueryData<InfiniteData<Paged, string | null>>(q.queryKey) ?? data;
    const key = paging.itemKey as (i: unknown) => string;
    const seen = new Set(fresh.items.map(key));
    const old = now.pages.flatMap((p) => p.items);
    const last = fresh.items[fresh.items.length - 1];
    const cursor = paging.getCursor as (p: unknown) => string | null | undefined;
    // What was loaded after the fresh first page's last row still follows it, as one page that keeps the
    // cursor of the last loaded page. When that row was not loaded before, rows moved too far to join the
    // two: start again from the fresh first page.
    const at = last === undefined ? -1 : old.findIndex((it) => key(it) === key(last));
    const tail = at < 0 || cursor(fresh) == null ? [] : old.slice(at + 1).filter((it) => !seen.has(key(it)));
    const tailPage = now.pages[now.pages.length - 1];
    if (tail.length === 0 || now.pages.length < 2 || !tailPage) {
      qc.setQueryData(q.queryKey, { pages: [fresh], pageParams: [null] });
      return;
    }
    qc.setQueryData(q.queryKey, { pages: [fresh, { ...tailPage, items: tail }], pageParams: [null, now.pageParams[now.pageParams.length - 1] ?? null] });
  } catch {
    /* the next event refreshes it again */
  } finally {
    const again = refreshing.get(hash);
    refreshing.delete(hash);
    if (again) void refreshHead(qc, q);
  }
}

/**
 * Refresh everything under `["data"]` after something changed. Plain queries are invalidated; a paging query
 * (`useProcInfinite`) is not refetched page by page: its first page is re-read once with room for one more page,
 * the later pages the reader loaded are kept without the rows now in the first, and one nobody is looking at
 * is dropped (it loads fresh when next opened).
 */
export function invalidateData(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: ["data"], predicate: (q) => !(q.meta as { paging?: unknown } | undefined)?.paging });
  for (const q of qc.getQueryCache().findAll({ queryKey: ["data", "inf"] })) {
    if (!(q.meta as { paging?: unknown } | undefined)?.paging) continue;
    if (q.isActive()) void refreshHead(qc, q);
    else qc.removeQueries({ queryKey: q.queryKey, exact: true });
  }
}
