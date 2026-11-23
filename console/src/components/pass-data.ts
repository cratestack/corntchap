import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import type { ActivityView, PassMessageView, PassPage, StationView, WokenRef } from "@corntchap/api";
import { client } from "@/lib/api";
import { useAgents, useProc, type AgentRow } from "@/lib/data";
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

/**
 * One conversation of a pot's pass, newest 100 messages. Reading marks them read on the server, so the pot
 * summaries (the unread badges) are refreshed after each read. Lives under ["data"], so `pass.posted` live
 * events (which invalidate everything) refetch it.
 */
export function usePass(pot: string, withWho: string, onRead: () => void) {
  const q = useQuery({
    queryKey: ["data", "pass", pot, withWho],
    queryFn: async (): Promise<PassPage> => {
      const page = await client.procedures.passRead({ project: pot, chef: withWho, limit: 100 });
      if (page.messages.length > 0) onRead();
      return page;
    },
    staleTime: 1500,
    placeholderData: (prev) => prev,
  });
  // while another conversation loads, `latest` still carries the unread counts; the messages are not shown
  return { page: q.isPlaceholderData ? undefined : q.data, latest: q.data, isLoading: q.isLoading || q.isPlaceholderData, error: q.error as unknown };
}

// ---------------------------------------------------------------- previews

export interface Preview {
  at: string;
  from: string | null;
  body: string;
}

export const previewKey = (pot: string, conv: string) => ["pass-prev", pot, conv] as const;

export const toPreview = (m: PassMessageView | undefined): Preview | null => (m ? { at: m.at, from: m.from ?? null, body: m.body } : null);

/**
 * The last message of each conversation, for the list. A read marks what it returns as read, so a preview is only
 * fetched for a conversation with nothing unread (it changes nothing there). One that has unread messages shows
 * "N new" instead; once you open it, the thread itself keeps its preview fresh (see PassView). Kept out of
 * ["data"] so live events do not refetch every conversation.
 */
export function usePreviews(pot: string, convs: { key: string; unread: number; active: boolean }[]) {
  const results = useQueries({
    queries: convs.map((c) => ({
      queryKey: previewKey(pot, c.key),
      queryFn: async (): Promise<Preview | null> => {
        const page = await client.procedures.passRead({ project: pot, chef: c.key, limit: 1 });
        return toPreview(page.messages[page.messages.length - 1]);
      },
      enabled: c.unread === 0 && !c.active,
      staleTime: Infinity,
    })),
  });
  const stamp = results.map((r) => r.dataUpdatedAt).join(",");
  const keys = convs.map((c) => c.key).join(",");
  return useMemo(() => {
    const m = new Map<string, Preview | null>();
    convs.forEach((c, i) => {
      const d = results[i]?.data;
      if (d !== undefined) m.set(c.key, d);
    });
    return m;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stamp, keys]);
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
  row: AgentRow;
}

export function useCooks(now: number) {
  const agents = useAgents();
  const bucket = Math.floor(now / 30_000);
  const byName = useMemo(() => {
    const m = new Map<string, CookInfo>();
    for (const a of agents.data ?? []) {
      m.set(a.name, {
        name: a.name,
        agentKind: KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind ?? "Cook",
        cloud: a.runtime === "cloud",
        row: a,
        state: cookState(a, bucket * 30_000),
      });
    }
    return m;
  }, [agents.data, bucket]);
  return { byName, agents: agents.data, isLoading: agents.isLoading };
}

// ---------------------------------------------------------------- wake status

export interface WakeSignals {
  stations: StationView[];
  activity: ActivityView[];
}

/** What the pot's stations and recent activity say about who is working right now. */
export function useWakeSignals(pot: string, enabled: boolean): WakeSignals {
  const stations = useProc("stationList", { project: pot, cook: null }, { enabled });
  const activity = useProc("activityList", { project: pot, station: null, task: null, limit: 100 }, { enabled });
  return { stations: stations.data ?? [], activity: activity.data ?? [] };
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
