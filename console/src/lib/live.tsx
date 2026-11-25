import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { ChangeView } from "@corntchap/api";
import { CONSOLE_HEADER, client, errorStatus, redirectToLogin } from "./api";
import { useInvalidateAll } from "./data";

/** How many events the live feed keeps; the activity page reads older ones from `eventPage`. */
export const KEEP = 400;
/** Coalesce bursts of events (a claim emits several) into one refetch. */
const INVALIDATE_MS = 250;

export type LiveStatus = "connecting" | "live" | "offline";

interface LiveState {
  events: ChangeView[];
  status: LiveStatus;
  /** True once the first full catch-up finished (so the feed is not shown half-empty). */
  ready: boolean;
}

const Ctx = createContext<LiveState>({ events: [], status: "connecting", ready: false });

/**
 * Catches up once with `changesSince`, then follows `/api/v1/events/stream` (SSE read through `fetch`, so the
 * console header and session cookie go along), resuming from the last seen event after any disconnect. New
 * events extend the activity feed and refresh every query through `invalidateData` (all data queries are small, so
 * touching everything is cheaper than mapping event kinds to resources; paging lists refresh their first page).
 */
export function LiveProvider({ children }: { children: ReactNode }) {
  const [events, setEvents] = useState<ChangeView[]>([]);
  const [status, setStatus] = useState<LiveStatus>("connecting");
  const [ready, setReady] = useState(false);
  const cursor = useRef(0);
  const primed = useRef(false);
  const invalidate = useInvalidateAll();

  useEffect(() => {
    const abort = new AbortController();
    let flush: ReturnType<typeof setTimeout> | undefined;
    let pending: ChangeView[] = [];

    const push = (e: ChangeView) => {
      if (e.seq <= cursor.current) return;
      cursor.current = e.seq;
      pending.push(e);
      clearTimeout(flush);
      flush = setTimeout(() => {
        const batch = pending;
        pending = [];
        setEvents((prev) => [...prev, ...batch].slice(-KEEP));
        void invalidate();
      }, INVALIDATE_MS);
    };

    async function catchUp() {
      const fresh: ChangeView[] = [];
      for (let i = 0; i < 30; i++) {
        const feed = await client.procedures.changesSince({ cursor: cursor.current, limit: 500 });
        if (feed.events.length === 0) break;
        fresh.push(...feed.events);
        cursor.current = feed.next_cursor;
        if (feed.events.length < 500) break;
      }
      setEvents((prev) => [...prev, ...fresh].slice(-KEEP));
      primed.current = true;
      setReady(true);
    }

    async function follow() {
      const res = await fetch(`/api/v1/events/stream?cursor=${cursor.current}`, {
        headers: { ...CONSOLE_HEADER, accept: "text/event-stream" },
        credentials: "same-origin",
        signal: abort.signal,
      });
      if (res.status === 401) {
        redirectToLogin();
        return;
      }
      if (!res.ok || !res.body) throw new Error(`event stream: HTTP ${res.status}`);
      setStatus("live");
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buf = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) return;
        buf += value.replace(/\r\n/g, "\n");
        let i;
        while ((i = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, i);
          buf = buf.slice(i + 2);
          let id = "";
          let data = "";
          for (const line of frame.split("\n")) {
            if (line.startsWith("id:")) id = line.slice(3).trim();
            else if (line.startsWith("data:")) data += line.slice(5).trim();
          }
          if (id && data) {
            try {
              push(JSON.parse(data) as ChangeView);
            } catch {
              /* a malformed frame is skipped; the next one carries on */
            }
          }
        }
      }
    }

    void (async () => {
      let delay = 1000;
      while (!abort.signal.aborted) {
        try {
          if (!primed.current) await catchUp();
          await follow();
          delay = 1000;
        } catch (e) {
          if (abort.signal.aborted) return;
          if (errorStatus(e) === 401) {
            redirectToLogin();
            return;
          }
        }
        if (abort.signal.aborted) return;
        setStatus("offline");
        await new Promise((r) => setTimeout(r, delay));
        delay = Math.min(delay * 2, 15000);
      }
    })();

    return () => {
      abort.abort();
      clearTimeout(flush);
    };
  }, [invalidate]);

  const value = useMemo(() => ({ events, status, ready }), [events, status, ready]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export const useLive = () => useContext(Ctx);
