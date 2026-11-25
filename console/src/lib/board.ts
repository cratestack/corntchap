import { useMemo } from "react";
import type { TaskPage, TaskView } from "@corntchap/api";
import type { Bucket } from "./format";
import { useProc } from "./data";
import { useProcInfinite } from "./paging";

/** The filters the server applies (taskPage): the cook (`holder`) and the column (`status`) too. */
export interface BoardFilters {
  label?: string;
  menu?: string;
  owner?: string;
  repo?: string;
  q?: string;
  status?: string;
  holder?: string;
}

/** Tasks per page of a column (the server allows up to 200). */
const PAGE = 25;
/** Tasks the filter chips are read from: the best of what is in play, unfiltered. */
const OPTIONS = 200;

export interface BucketMore {
  has: boolean;
  loading: boolean;
  load: () => void;
}

export interface BoardData {
  loading: boolean;
  error: unknown;
  groups: Record<Bucket, TaskView[]>;
  /** What the server counts per column, filters applied. */
  totals: Partial<Record<Bucket, number>>;
  more: Partial<Record<Bucket, BucketMore>>;
  /** Nothing on the board at all (and no filter hiding it). */
  bare: boolean;
  /** Tasks the filter chips offer (the board's best, unfiltered, and what the columns hold). */
  options: TaskView[];
}

const cursorOf = (p: TaskPage) => p.nextCursor;

/** The board's columns, in the order the board shows them. */
export const COLUMNS: Bucket[] = ["ready", "claimed", "waiting", "done", "cancelled"];

const IN_PLAY = new Set<Bucket>(["ready", "claimed", "waiting"]);

const changedAt = (t: TaskView) => Date.parse(t.updated_at) || 0;

/**
 * Each task in one column only. The columns are separate lists refreshed one by one, and a served or tossed column
 * refreshes its head only: a task restored from deep in Tossed is in Ready before it leaves Tossed. The copy changed
 * last wins (on a tie, the first column in board order).
 */
export function oneColumnEach(groups: Record<Bucket, TaskView[]>): Record<Bucket, TaskView[]> {
  const best = new Map<string, { b: Bucket; at: number }>();
  for (const b of COLUMNS) {
    for (const t of groups[b]) {
      const at = changedAt(t);
      const cur = best.get(t.id);
      if (!cur || at > cur.at) best.set(t.id, { b, at });
    }
  }
  const out = {} as Record<Bucket, TaskView[]>;
  for (const b of COLUMNS) {
    const list = groups[b];
    out[b] = list.some((t) => best.get(t.id)?.b !== b) ? list.filter((t) => best.get(t.id)?.b === b) : list;
  }
  return out;
}

/**
 * A pot's board from `taskPage`: every column is its own infinite query (a page at a time, as far as the reader
 * scrolls), with the server's total for it. The label, menu, owner, repo, text, cook and status filters are the
 * server's too, so a count and the cards under it always agree.
 */
export function useBoardData(pot: string, f: BoardFilters, enabled: boolean): BoardData {
  const args = (bucket: string) => ({
    project: pot,
    bucket,
    label: f.label || null,
    menu: f.menu || null,
    owner: f.owner || null,
    repo: f.repo || null,
    q: f.q?.trim() || null,
    holder: f.holder || null,
    status: f.status || null,
  });
  // a column the status filter rules out is not asked for
  const on = (b: Bucket) => enabled && (!f.status || f.status === b);
  // the columns in play are sorted by priority, so a task can leave or join one anywhere in it: a live refresh
  // re-reads all that is loaded there. Served and tossed take changes at the top (newest change first).
  const opts = (b: Bucket) => ({ getCursor: cursorOf, pageSize: PAGE, maxLimit: 200, maxPages: Infinity, enabled: on(b), refreshLoaded: IN_PLAY.has(b) });
  const ready = useProcInfinite("taskPage", args("ready"), opts("ready"));
  const claimed = useProcInfinite("taskPage", args("claimed"), opts("claimed"));
  const waiting = useProcInfinite("taskPage", args("waiting"), opts("waiting"));
  const done = useProcInfinite("taskPage", args("done"), opts("done"));
  const cancelled = useProcInfinite("taskPage", args("cancelled"), opts("cancelled"));
  const cols = { ready, claimed, waiting, done, cancelled };

  const unfiltered = useProc(
    "taskPage",
    { project: pot, bucket: "active", label: null, menu: null, owner: null, repo: null, q: null, holder: null, status: null, page: { limit: OPTIONS, cursor: null } },
    { enabled },
  );

  const groups = useMemo(
    () => oneColumnEach({ ready: ready.items, claimed: claimed.items, waiting: waiting.items, done: done.items, cancelled: cancelled.items }),
    [ready.items, claimed.items, waiting.items, done.items, cancelled.items],
  );
  const anyFilter = !!(f.label || f.menu || f.owner || f.repo || f.q?.trim() || f.status || f.holder);
  const loading = COLUMNS.some((b) => on(b) && cols[b].isLoading);
  const error = COLUMNS.map((b) => cols[b].error).find(Boolean);
  const more = (q: typeof done): BucketMore => ({ has: !!q.hasNextPage, loading: q.isFetchingNextPage, load: () => void (q.hasNextPage && !q.isFetchingNextPage && q.fetchNextPage()) });
  const totals: Partial<Record<Bucket, number>> = {};
  for (const b of COLUMNS) {
    const n = on(b) ? cols[b].total : 0;
    if (n != null) totals[b] = n;
  }
  const optionTasks = useMemo(() => {
    const all = [...(unfiltered.data?.items ?? []), ...ready.items, ...claimed.items, ...waiting.items, ...done.items, ...cancelled.items];
    return [...new Map(all.map((t) => [t.id, t])).values()];
  }, [unfiltered.data, ready.items, claimed.items, waiting.items, done.items, cancelled.items]);
  return {
    loading,
    error,
    groups,
    totals,
    more: { ready: more(ready), claimed: more(claimed), waiting: more(waiting), done: more(done), cancelled: more(cancelled) },
    bare: !anyFilter && !loading && COLUMNS.every((b) => (cols[b].total ?? 0) === 0),
    options: optionTasks,
  };
}
