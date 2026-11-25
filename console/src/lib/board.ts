import { useMemo } from "react";
import type { TaskPage, TaskView } from "@corntchap/api";
import type { Bucket } from "./format";
import { useProcInfinite } from "./paging";
import { groupTasks } from "@/components/task-views";

/** The filters the server applies (taskPage); `status` and `holder` are applied to what is loaded. */
export interface BoardFilters {
  label?: string;
  menu?: string;
  owner?: string;
  repo?: string;
  q?: string;
  status?: string;
  holder?: string;
}

/** Tasks per page of the served and tossed columns. */
const CLOSED_PAGE = 25;
/** One request holds every task still in play (open, claimed, blocked) up to this many. */
const ACTIVE_PAGE = 500;

export interface BucketMore {
  has: boolean;
  loading: boolean;
  load: () => void;
}

export interface BoardData {
  loading: boolean;
  error: unknown;
  groups: Record<Bucket, TaskView[]>;
  /** What the server counts per column, for the columns it pages (served, tossed). */
  totals: Partial<Record<Bucket, number>>;
  more: Partial<Record<Bucket, BucketMore>>;
  /** Nothing on the board at all (and no filter hiding it). */
  bare: boolean;
  /** Every task loaded without the filters: what the filter chips offer. */
  options: TaskView[];
}

const cursorOf = (p: TaskPage) => p.next_cursor;

/**
 * A pot's board from `taskPage`: every open, claimed and blocked task in one go, and the served and tossed
 * columns a page at a time (newest first), each with the server's total.
 */
export function useBoardData(pot: string, f: BoardFilters, enabled: boolean): BoardData {
  const args = (bucket: "active" | "done" | "cancelled", filtered: boolean) => ({
    project: pot,
    bucket,
    label: (filtered && f.label) || null,
    menu: (filtered && f.menu) || null,
    owner: (filtered && f.owner) || null,
    repo: (filtered && f.repo) || null,
    q: (filtered && f.q?.trim()) || null,
  });
  const opts = (pageSize: number, maxLimit: number) => ({ getCursor: cursorOf, pageSize, maxLimit, enabled });
  const active = useProcInfinite("taskPage", args("active", true), opts(ACTIVE_PAGE, 500));
  const done = useProcInfinite("taskPage", args("done", true), opts(CLOSED_PAGE, 200));
  const cancelled = useProcInfinite("taskPage", args("cancelled", true), opts(CLOSED_PAGE, 200));
  // the unfiltered active tasks (the same query as `active` while no filter is set)
  const options = useProcInfinite("taskPage", args("active", false), opts(ACTIVE_PAGE, 500));

  const { status, holder } = f;
  const groups = useMemo(() => {
    const g = groupTasks(active.items);
    g.done = done.items;
    g.cancelled = cancelled.items;
    const keep = (t: TaskView) => !holder || t.claimed_by === holder;
    for (const b of Object.keys(g) as Bucket[]) g[b] = status && status !== b ? [] : g[b].filter(keep);
    return g;
  }, [active.items, done.items, cancelled.items, status, holder]);

  const anyFilter = !!(f.label || f.menu || f.owner || f.repo || f.q?.trim() || status || holder);
  const loading = active.isLoading || done.isLoading || cancelled.isLoading;
  const error = active.error ?? done.error ?? cancelled.error;
  const counted = !holder && !status;
  const more = (q: typeof done): BucketMore => ({ has: !!q.hasNextPage, loading: q.isFetchingNextPage, load: () => void (q.hasNextPage && !q.isFetchingNextPage && q.fetchNextPage()) });
  const optionTasks = useMemo(() => [...options.items, ...done.items, ...cancelled.items], [options.items, done.items, cancelled.items]);
  return {
    loading,
    error,
    groups,
    totals: counted ? { ...(done.total != null ? { done: done.total } : {}), ...(cancelled.total != null ? { cancelled: cancelled.total } : {}) } : {},
    more: { ready: more(active), claimed: more(active), waiting: more(active), done: more(done), cancelled: more(cancelled) },
    bare: !anyFilter && !loading && (active.total ?? 0) === 0 && (done.total ?? 0) === 0 && (cancelled.total ?? 0) === 0,
    options: optionTasks,
  };
}
