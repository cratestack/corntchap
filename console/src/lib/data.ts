import { useCallback } from "react";
import { useCustom, useCustomMutation, useDataProvider } from "@refinedev/core";
import { keepPreviousData, useQuery, useQueryClient, useQueries } from "@tanstack/react-query";
import type { CorntchapApiClient, ProjectSummary, TaskView } from "@corntchap/api";
import { v1 } from "./api";
import { invalidateData } from "./paging";

type Procs = CorntchapApiClient["procedures"];
export type ProcName = keyof Procs;
export type ProcArgs<N extends ProcName> = Parameters<Procs[N]>[0];
export type ProcOut<N extends ProcName> = Awaited<ReturnType<Procs[N]>>;

/**
 * Query a cratestack procedure through Refine's `custom` (the data provider's escape hatch for procedures).
 * Everything lives under the `["data"]` query-key prefix, so the live poller can invalidate it in one call.
 */
export function useProc<N extends ProcName>(name: N, args: ProcArgs<N>, opts: { enabled?: boolean; refetchMs?: number; keepPrevious?: boolean } = {}) {
  const { query } = useCustom({
    url: "",
    method: "get",
    meta: { procedure: name },
    config: { payload: args },
    queryOptions: { enabled: opts.enabled ?? true, staleTime: 1500, refetchInterval: opts.refetchMs ?? false, ...(opts.keepPrevious ? { placeholderData: keepPreviousData } : {}) },
    errorNotification: false,
  });
  const data = (query.data as { data?: unknown } | undefined)?.data as ProcOut<N> | undefined;
  return {
    data,
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    error: query.error as unknown,
    refetch: query.refetch,
  };
}

/** Refresh every Refine / console query (the live poller and every mutation call this); see `invalidateData`. */
export function useInvalidateAll() {
  const qc = useQueryClient();
  return useCallback(() => invalidateData(qc), [qc]);
}

/** Call a mutating procedure (claim, complete, create, ...), then refresh everything. */
export function useProcCall() {
  const { mutateAsync } = useCustomMutation();
  const invalidate = useInvalidateAll();
  return useCallback(
    async <N extends ProcName>(name: N, args: ProcArgs<N>): Promise<ProcOut<N>> => {
      const res = await mutateAsync({
        url: "",
        method: "post",
        values: args as Record<string, unknown>,
        meta: { procedure: name },
        successNotification: false,
        errorNotification: false,
      });
      void invalidate();
      return (res as { data: unknown }).data as ProcOut<N>;
    },
    [mutateAsync, invalidate],
  );
}

/** GET on a hand-written /api/v1 endpoint (admin operations that /api/v2 does not have). */
export function useV1<T>(path: string, opts: { enabled?: boolean } = {}) {
  const q = useQuery({
    queryKey: ["data", "v1", path],
    queryFn: () => v1<T>("GET", path),
    enabled: opts.enabled ?? true,
    staleTime: 1500,
  });
  return { data: q.data, isLoading: q.isLoading, error: q.error as unknown, refetch: q.refetch };
}

/** POST/DELETE/... on /api/v1, then refresh everything. */
export function useV1Call() {
  const invalidate = useInvalidateAll();
  return useCallback(
    async <T = unknown>(method: string, path: string, body?: unknown): Promise<T> => {
      const out = await v1<T>(method, path, body);
      void invalidate();
      return out;
    },
    [invalidate],
  );
}

export function useProjects() {
  return useProc("projectOverview", {});
}

/** Rows per request when reading a pot's claimed tasks (the server's most). */
const CLAIMED_PAGE = 200;

/**
 * Live (claimed) tasks across every pot: who is cooking what. Each pot's claimed tasks are read through `taskPage`
 * (`status` claimed), page after page to the end: a cook holds a few leases, so what is claimed stays small however
 * long the pot's lists are.
 */
export function useClaimedTasks(projects: ProjectSummary[] | undefined) {
  const dp = useDataProvider();
  const results = useQueries({
    queries: (projects ?? []).map((p) => ({
      queryKey: ["data", "claimed", p.key],
      queryFn: async () => {
        const tasks: TaskView[] = [];
        let cursor: string | null = null;
        do {
          const res: { data: unknown } = await dp().custom!({
            url: "",
            method: "get",
            meta: { procedure: "taskPage" },
            payload: { project: p.key, status: "claimed", page: { limit: CLAIMED_PAGE, cursor } },
          });
          const page = res.data as { items: TaskView[]; nextCursor?: string | null };
          tasks.push(...page.items);
          cursor = page.nextCursor ?? null;
        } while (cursor);
        return tasks;
      },
      staleTime: 1500,
    })),
  });
  const tasks: TaskView[] = results.flatMap((r) => r.data ?? []);
  return { tasks, isLoading: results.some((r) => r.isLoading) };
}

/** Open-question counts per pot, for the sidebar badges (from the pot summaries, no extra requests). */
export function useOpenQuestionCounts(projects: ProjectSummary[] | undefined): Record<string, number> {
  const out: Record<string, number> = {};
  for (const p of projects ?? []) if (p.open_questions > 0) out[p.key] = p.open_questions;
  return out;
}

// ---- my work ----

/**
 * What the signed-in person owns, holds and is waited on for, across every pot (the `my_work` tool).
 * Under `["data"]`, so the live poller refreshes it; the sidebar badge and the page share one query.
 */
export function useMyWork(project?: string) {
  return useProc("myWork", project ? { project } : {}, { refetchMs: 30_000 });
}
