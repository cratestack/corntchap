import { useCallback } from "react";
import { useCustom, useCustomMutation, useDataProvider } from "@refinedev/core";
import { useQuery, useQueryClient, useQueries } from "@tanstack/react-query";
import type { CorntchapApiClient, ProjectSummary, QuestionView, TaskView } from "@corntchap/api";
import { v1 } from "./api";

type Procs = CorntchapApiClient["procedures"];
export type ProcName = keyof Procs;
export type ProcArgs<N extends ProcName> = Parameters<Procs[N]>[0];
export type ProcOut<N extends ProcName> = Awaited<ReturnType<Procs[N]>>;

/**
 * Query a cratestack procedure through Refine's `custom` (the data provider's escape hatch for procedures).
 * Everything lives under the `["data"]` query-key prefix, so the live poller can invalidate it in one call.
 */
export function useProc<N extends ProcName>(name: N, args: ProcArgs<N>, opts: { enabled?: boolean } = {}) {
  const { query } = useCustom({
    url: "",
    method: "get",
    meta: { procedure: name },
    config: { payload: args },
    queryOptions: { enabled: opts.enabled ?? true, staleTime: 1500 },
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

/** Invalidate every Refine / console query (the live poller and every mutation call this). */
export function useInvalidateAll() {
  const qc = useQueryClient();
  return useCallback(() => qc.invalidateQueries({ queryKey: ["data"] }), [qc]);
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

/** Live (claimed) tasks across every pot: who is cooking what. */
export function useClaimedTasks(projects: ProjectSummary[] | undefined) {
  const dp = useDataProvider();
  const results = useQueries({
    queries: (projects ?? []).map((p) => ({
      queryKey: ["data", "claimed", p.key],
      queryFn: async () => {
        const res = await dp().custom!({
          url: "",
          method: "get",
          meta: { procedure: "projectTasks" },
          payload: { project: p.key, status: "claimed" },
        });
        return (res as { data: TaskView[] }).data;
      },
      staleTime: 1500,
    })),
  });
  const tasks: TaskView[] = results.flatMap((r) => r.data ?? []);
  return { tasks, isLoading: results.some((r) => r.isLoading) };
}

// ---- /api/v1 shapes (hand-written endpoints; no generated types) ----

export interface AgentRow {
  name: string;
  kind: string;
  agent_kind: string | null;
  runtime: string | null;
  max_leases: number;
  owner: string | null;
  /** Cooks: what a person granted beyond working tasks (plan, manage, admin). */
  grants?: string[];
  disabled: boolean;
  last_seen_at: string | null;
  busy: number;
}

export interface UserRow {
  name: string;
  login: string | null;
  is_admin: boolean;
  linked: boolean;
  disabled: boolean;
}

export interface TeamRow {
  name: string;
  role: string | null;
  members: number;
  projects: string[];
}

export interface MemberRow {
  name: string;
  kind: string;
  role: string;
}

export interface WakeTargetRow {
  id: string;
  type: string;
  projects: string[];
  labels: string[];
  config: Record<string, unknown>;
}

export function useAgents() {
  return useV1<AgentRow[]>("/agents");
}

// ---- the question pool ----

/**
 * A pot's questions by status. The query key is shared with the sidebar's counts, so the open list is fetched once.
 * Everything sits under `["data"]`, so the live poller refreshes it on question.* events.
 */
export function useQuestions(pot: string, status: "open" | "all", enabled = true) {
  const dp = useDataProvider();
  const q = useQuery({
    queryKey: ["data", "questions", pot, status],
    queryFn: async () => {
      const res = await dp().custom!({ url: "", method: "get", meta: { procedure: "questionList" }, payload: { project: pot, status } });
      return (res as { data: QuestionView[] }).data;
    },
    enabled: enabled && !!pot,
    staleTime: 1500,
  });
  return { data: q.data, isLoading: q.isLoading, error: q.error as unknown };
}

/** Open-question counts per pot, for the sidebar badges (from the pot summaries, no extra requests). */
export function useOpenQuestionCounts(projects: ProjectSummary[] | undefined): Record<string, number> {
  const out: Record<string, number> = {};
  for (const p of projects ?? []) if (p.open_questions > 0) out[p.key] = p.open_questions;
  return out;
}
