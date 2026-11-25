import { useCallback } from "react";
import { useCustom, useCustomMutation, useDataProvider } from "@refinedev/core";
import { keepPreviousData, useQuery, useQueryClient, useQueries } from "@tanstack/react-query";
import type { CorntchapApiClient, FileView, ProjectSummary, TaskView, ViewOut } from "@corntchap/api";
import { client, v1 } from "./api";
import { invalidateData, usePagedQuery, useProcInfinite } from "./paging";

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

/**
 * Read a generated model (`/api/v2/<model>`): the server applies the model's read policy, so these
 * lists hold only what the signed-in person may see. Lives under ["data"] like every other query.
 */
function useModel<T>(key: unknown[], fn: () => Promise<T>, enabled = true) {
  const q = useQuery({ queryKey: ["data", "model", ...key], queryFn: fn, enabled, staleTime: 1500 });
  return { data: q.data, isLoading: q.isLoading, error: q.error as unknown, refetch: q.refetch };
}

const num = (v: bigint | number | null | undefined) => (v === null || v === undefined ? null : Number(v));

/** A team's chefs (people and cooks) with their role, from the TeamMember model. */
export function useTeamChefs(team: string | null | undefined, opts: { enabled?: boolean } = {}) {
  return useModel(
    ["team-members", team],
    async (): Promise<MemberRow[]> => {
      const rows = await client.teamMembers.list({ query: { filters: { "team.name": team ?? "" }, include: ["principal"] } });
      return rows
        .filter((m) => m.principal?.name)
        .map((m) => ({ name: m.principal!.name!, display_name: m.principal!.displayName ?? null, kind: m.principal!.kind ?? "user", role: m.role ?? "member" }))
        .sort((a, b) => a.name.localeCompare(b.name));
    },
    !!team && (opts.enabled ?? true),
  );
}

/** Files per page of a pot's file list. */
const FILES_PAGE = 50;

/** A pot's files (newest first, a page at a time), from the File model's keyset-paged list. */
export function usePotFiles(pot: string) {
  return usePagedQuery(
    ["files", pot],
    async (limit, cursor): Promise<{ items: FileView[]; total: number | null; nextCursor: string | null }> => {
      const page = await client.files.list({
        query: {
          filters: { "project.key": pot },
          include: ["project", "task", "question", "creator"],
          sort: ["-createdAt", "-id"],
          limit,
          cursor: cursor ?? "",
        },
      });
      const items = page.items.map((f): FileView => {
        const key = f.project?.key ?? pot;
        const id = `${key}-F${num(f.number)}`;
        return {
          id,
          project: key,
          name: f.name ?? "",
          content_type: f.contentType ?? "application/octet-stream",
          size: num(f.size),
          size_expected: num(f.sizeExpected),
          status: f.status ?? "uploading",
          url: `/files/${id}/${encodeURIComponent(f.name ?? "")}`,
          task: f.task ? `${key}-${num(f.task.number)}` : null,
          question: f.question ? `${key}-Q${num(f.question.number)}` : null,
          created_by: f.creator?.name ?? null,
          created_at: f.createdAt ?? "",
          ready_at: f.readyAt ?? null,
          error: f.error ?? null,
        };
      });
      return { items, total: page.totalCount, nextCursor: page.pageInfo.nextCursor ?? null };
    },
    { getCursor: (p) => p.nextCursor, pageSize: FILES_PAGE, maxLimit: 1000 },
  );
}

/** A pot's saved views in tab order, from the PotView model. */
export function usePotViews(pot: string, opts: { enabled?: boolean } = {}) {
  return useModel(
    ["views", pot],
    async (): Promise<ViewOut[]> => {
      const rows = await client.potViews.list({ query: { filters: { "project.key": pot }, include: ["creator"], sort: ["position"] } });
      return rows.map((v) => ({
        key: v.key ?? "",
        name: v.name ?? "",
        description: v.description ?? null,
        layout: v.layout ?? "list",
        where: v.whereSql ?? "true",
        order: v.orderSql ?? null,
        position: num(v.position) ?? 0,
        created_by: v.creator?.name ?? null,
        updated_at: v.updatedAt ?? "",
      }));
    },
    opts.enabled ?? true,
  );
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
  id: string;
  name: string;
  kind: string;
  agent_kind: string | null;
  runtime: string | null;
  max_leases: number;
  owner: string | null;
  /** Cooks: what a person granted beyond working tasks (plan, manage, admin). */
  grants?: string[];
  /** Repositories the cook handles per pot (a pot not listed: all of its repositories). */
  repo_scopes?: { project: string; repos: string[] }[];
  /** Teams the cook is in (empty: it sees no pot). */
  teams?: string[];
  disabled: boolean;
  last_seen_at: string | null;
  busy: number;
  /** Cloud targets only: takes ready work, opens a pull request and stops (see the README). */
  fireAndForget?: boolean;
}

export interface TeamRow {
  name: string;
  role: string | null;
  members: number;
  projects: string[];
}

export interface MemberRow {
  name: string;
  display_name?: string | null;
  kind: string;
  role: string;
  /** Cloud targets only: takes ready work, opens a pull request and stops. */
  fireAndForget?: boolean;
}

export function useAgents() {
  return useV1<AgentRow[]>("/agents");
}

/** Open-question counts per pot, for the sidebar badges (from the pot summaries, no extra requests). */
export function useOpenQuestionCounts(projects: ProjectSummary[] | undefined): Record<string, number> {
  const out: Record<string, number> = {};
  for (const p of projects ?? []) if (p.open_questions > 0) out[p.key] = p.open_questions;
  return out;
}

// ---- reviews ----

/**
 * A pot's reviews, a page at a time: the ones waiting on you (`mine`), or all of them (open ones first).
 * Live events refresh the first page (see `invalidateData`).
 */
export function useReviews(pot: string, which: "mine" | "all", enabled = true) {
  const args =
    which === "mine"
      ? { project: pot, mine: true, state: "requested" as string | null }
      : { project: pot, mine: null, state: "all" as string | null };
  const q = useProcInfinite("reviewPage", args, { getCursor: (p) => p.nextCursor, enabled: enabled && !!pot });
  return { data: q.items, isLoading: q.isLoading, error: q.error, hasNextPage: q.hasNextPage, fetchNextPage: q.fetchNextPage, isFetchingNextPage: q.isFetchingNextPage };
}

// ---- my work ----

/**
 * What the signed-in person owns, holds and is waited on for, across every pot (the `my_work` tool).
 * Under `["data"]`, so the live poller refreshes it; the sidebar badge and the page share one query.
 */
export function useMyWork(project?: string) {
  return useProc("myWork", project ? { project } : {}, { refetchMs: 30_000 });
}
