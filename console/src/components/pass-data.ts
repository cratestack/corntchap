import { useCallback, useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ActivityView, PassConversationView, PassMessageView, PassPage, StationView, WokenRef } from "@corntchap/api";
import { client } from "@/lib/api";
import { useListQuery } from "@/lib/list-query";
import type { ListSource } from "@/lib/listbox";
import { useCooksSource } from "@/lib/sources";
import { KIND_LABEL } from "@/lib/snippets";
import { cookState, type CookState } from "./bits";

/** The conversation key of the pot's own pass (the `chef` argument of passRead). */
export const POT = "pot";

/** "doug · quiet-planet" for a woken session, "doug" for a cook woken through its wake target. */
export function wokenName(w: WokenRef): string {
  const where = w.label ?? w.station;
  const name = where ? `${w.cook} · ${where}` : w.cook;
  // a cloud cook answers once in a fresh run, then stops
  return w.cloud ? `${name} (cloud run, fire and forget)` : name;
}

export const isEveryone = (body: string) => /(^|\s)@(all|everyone)\b/i.test(body);

/** Older messages asked for at a time when scrolling back. */
const OLDER_PAGE = 50;

/** Messages by seq, once each, oldest first. */
function bySeq(a: PassMessageView[], b: PassMessageView[]): PassMessageView[] {
  const m = new Map<number, PassMessageView>();
  for (const x of a) m.set(x.seq, x);
  for (const x of b) m.set(x.seq, x);
  return [...m.values()].sort((x, y) => x.seq - y.seq);
}

/**
 * One conversation of a pot's pass: the newest 100 messages, and older ones on request (`loadOlder`, through
 * passRead's `before`, which marks nothing read). Reading marks the newest messages read on the server, so the
 * pot summaries (the unread badges) are refreshed after each read. Lives under ["data"], so `pass.posted` live
 * events (which invalidate everything) refetch the newest messages; those are merged into what is loaded, so the
 * older messages already scrolled back to stay.
 */
export function usePass(pot: string, withWho: string, onRead: () => void) {
  const qc = useQueryClient();
  const key = useMemo(() => ["data", "pass", pot, withWho] as const, [pot, withWho]);
  const q = useQuery({
    queryKey: key,
    queryFn: async (): Promise<PassPage> => {
      const page = await client.procedures.passRead({ project: pot, chef: withWho, limit: 100 });
      if (page.messages.length > 0) onRead();
      const had = qc.getQueryData<PassPage>(key);
      if (!had || had.messages.length === 0) return page;
      // older messages loaded earlier stay; whether still older ones exist is the oldest loaded one's answer
      const reachesBack = (had.messages[0]?.seq ?? Infinity) < (page.messages[0]?.seq ?? Infinity);
      return { ...page, messages: bySeq(had.messages, page.messages), older: reachesBack ? had.older : page.older };
    },
    staleTime: 1500,
    placeholderData: (prev) => prev,
  });
  const older = useMutation({
    mutationFn: async () => {
      const have = qc.getQueryData<PassPage>(key);
      const first = have?.messages[0]?.seq;
      if (!have || first === undefined) return;
      const page = await client.procedures.passRead({ project: pot, chef: withWho, limit: OLDER_PAGE, before: first });
      qc.setQueryData<PassPage>(key, (cur) => (cur ? { ...cur, messages: bySeq(page.messages, cur.messages), older: page.older } : cur));
    },
  });
  const loadOlder = older.mutate;
  // while another conversation loads, `latest` still carries the unread counts; the messages are not shown
  const shown = q.isPlaceholderData ? undefined : q.data;
  return {
    page: shown,
    latest: q.data,
    isLoading: q.isLoading || q.isPlaceholderData,
    error: q.error as unknown,
    hasOlder: !!shown?.older,
    loadingOlder: older.isPending,
    loadOlder: useCallback(() => loadOlder(), [loadOlder]),
  };
}

// ---------------------------------------------------------------- conversations

/** The conversation list's own filters: the search text and "only the ones with something unread". */
export interface ConversationFilters {
  q?: string;
  unread?: boolean;
  enabled?: boolean;
}

/**
 * Your direct conversations in a pot, the most recent first (passConversationPage: the chef, the last message cut,
 * the unread count). A message moves its conversation to the top, so a live refresh restarts from the first page and
 * re-reads everything loaded (`refreshLoaded`): a conversation that moved up from past the cursor must not be lost.
 */
export function useConversations(pot: string, f: ConversationFilters = {}): ListSource<PassConversationView> {
  return useListQuery(
    "passConversationPage",
    { project: pot, ...(f.unread ? { unread: true } : {}) },
    { q: f.q ?? "", refreshLoaded: true, enabled: f.enabled ?? true },
  );
}

type Cached = { pages: { items: PassConversationView[] }[]; pageParams: unknown[] };

/** A conversation read: its unread count is 0 in every loaded list of conversations (the server has marked it read). */
export function useMarkConversationRead() {
  const qc = useQueryClient();
  return useCallback(
    (chef: string) => {
      qc.setQueriesData<Cached>({ queryKey: ["data", "inf", "passConversationPage"] }, (data) => {
        if (!data) return data;
        let changed = false;
        const pages = data.pages.map((pg) => {
          if (!pg.items.some((c) => c.chef === chef && c.unread > 0)) return pg;
          changed = true;
          return { ...pg, items: pg.items.map((c) => (c.chef === chef ? { ...c, unread: 0 } : c)) };
        });
        return changed ? { ...data, pages } : data;
      });
    },
    [qc],
  );
}

// ---------------------------------------------------------------- previews

export interface Preview {
  at: string;
  from: string | null;
  body: string;
  /** The last message of a direct conversation is the viewer's own. */
  mine?: boolean;
}

export const previewKey = (pot: string, conv: string) => ["pass-prev", pot, conv] as const;

export const toPreview = (m: PassMessageView | undefined): Preview | null => (m ? { at: m.at, from: m.from ?? null, body: m.body } : null);

/** A conversation row's preview, from the list's own page (the last message cut to one line, and its time). */
export const conversationPreview = (c: PassConversationView): Preview | null => (c.preview ? { at: c.last_at, from: null, body: c.preview, mine: c.last_mine } : null);

/**
 * The last message of the pot's own pass, for the list (the direct conversations come with theirs). A read marks
 * what it returns as read, so it is only fetched when nothing is unread there (it changes nothing). Once you open
 * the pass, the thread itself keeps its preview fresh (see PassView). Kept out of ["data"] so live events do not
 * refetch it.
 */
export function usePotPreview(pot: string, enabled: boolean): Preview | null | undefined {
  return useQuery({
    queryKey: previewKey(pot, POT),
    queryFn: async (): Promise<Preview | null> => {
      const page = await client.procedures.passRead({ project: pot, chef: POT, limit: 1 });
      return toPreview(page.messages[page.messages.length - 1]);
    },
    enabled,
    staleTime: Infinity,
  }).data;
}

/** One line of a message for the list: markdown noise removed, whitespace collapsed. */
export function oneLine(body: string): string {
  return body
    .replace(/```[\s\S]*?```/g, " code ")
    .replace(/^>\s?/gm, "")
    .replace(/[*_`#]/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

// ---------------------------------------------------------------- cooks

export interface CookInfo {
  name: string;
  /** "Claude Code", "Codex" ... */
  agentKind: string;
  cloud: boolean;
  state: CookState;
}

/**
 * What the pass knows about cooks: the team's cooks as far as they are loaded (cookPage of the team), and the cook
 * of the open conversation (found by name). Someone not loaded yet just shows no presence.
 */
export function useCooks(now: number, team: string | null, open: string | null) {
  const inTeam = useCooksSource({ team: team ?? "", enabled: !!team });
  const named = useCooksSource({ q: open ?? "", enabled: !!open });
  const bucket = Math.floor(now / 30_000);
  const byName = useMemo(() => {
    const m = new Map<string, CookInfo>();
    for (const a of [...inTeam.items, ...named.items]) {
      if (m.has(a.name)) continue;
      m.set(a.name, {
        name: a.name,
        agentKind: KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind ?? "Cook",
        cloud: a.runtime === "cloud",
        state: cookState({ disabled: a.disabled, busy: a.busy, last_seen_at: a.last_seen_at ?? null }, bucket * 30_000),
      });
    }
    return m;
  }, [inTeam.items, named.items, bucket]);
  return { byName, isLoading: inTeam.isLoading };
}

// ---------------------------------------------------------------- wake status

export interface WakeSignals {
  stations: StationView[];
  activity: ActivityView[];
}

/** Rows of each read the signals look at: the pot's newest stations and its newest tool calls. */
const SIGNAL_ROWS = 100;

/** What the pot's newest stations and recent activity say about who is working right now (one page of each: a cook further back reads as not working). */
export function useWakeSignals(pot: string, enabled: boolean): WakeSignals {
  const stations = useListQuery("stationPage", { project: pot }, { pageSize: SIGNAL_ROWS, enabled });
  const activity = useListQuery("activityPage", { project: pot }, { pageSize: SIGNAL_ROWS, enabled });
  return useMemo(() => ({ stations: stations.items as StationView[], activity: activity.items as ActivityView[] }), [stations.items, activity.items]);
}

export type WakeState = "woken" | "working" | "replied" | "quiet";

const MIN = 60_000;

/**
 * Where a woken cook is, from what the client can see (the server records only who was woken):
 * replied  - the cook posted later in this conversation;
 * working  - within 30 minutes of the message, one of its sessions holds a lease, or it ran a tool in the last two minutes;
 * quiet    - nothing after 5 minutes (10 for a cloud run, which starts cold);
 * woken    - otherwise.
 * A reply in another conversation (a mention answered in a direct message) is not seen from here.
 */
export function wakeState(m: PassMessageView, w: WokenRef, messages: PassMessageView[], sig: WakeSignals, now: number): WakeState {
  const at = new Date(m.at).getTime();
  if (messages.some((x) => x.seq > m.seq && x.from === w.cook)) return "replied";
  const age = now - at;
  const holds = sig.stations.some((s) => s.cook === w.cook && s.holding.length > 0);
  const ran = sig.activity.some((a) => {
    const t = new Date(a.at).getTime();
    return a.cook === w.cook && t > at && now - t < 2 * MIN;
  });
  if ((holds || ran) && age < 30 * MIN) return "working";
  return age > (w.cloud ? 10 : 5) * MIN ? "quiet" : "woken";
}
