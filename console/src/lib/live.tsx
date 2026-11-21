import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { ChangeView } from "@corntchap/api";
import { client, errorStatus, redirectToLogin } from "./api";
import { useInvalidateAll } from "./data";

const POLL_MS = 3000;
const KEEP = 400;

export type LiveStatus = "connecting" | "live" | "offline";

interface LiveState {
  events: ChangeView[];
  status: LiveStatus;
  /** True once the first full catch-up finished (so the feed is not shown half-empty). */
  ready: boolean;
}

const Ctx = createContext<LiveState>({ events: [], status: "connecting", ready: false });

/**
 * Polls the `changesSince` procedure every ~3 s. New events extend the activity feed and invalidate every query
 * (all data queries are small, so touching everything is cheaper than mapping event kinds to resources).
 */
export function LiveProvider({ children }: { children: ReactNode }) {
  const [events, setEvents] = useState<ChangeView[]>([]);
  const [status, setStatus] = useState<LiveStatus>("connecting");
  const [ready, setReady] = useState(false);
  const cursor = useRef(0);
  const primed = useRef(false);
  const invalidate = useInvalidateAll();

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function poll() {
      try {
        const fresh: ChangeView[] = [];
        // drain: the server returns at most 500 events per call
        for (let i = 0; i < 30; i++) {
          const feed = await client.procedures.changesSince({ cursor: cursor.current, limit: 500 });
          if (feed.events.length === 0) break;
          fresh.push(...feed.events);
          cursor.current = feed.next_cursor;
          if (feed.events.length < 500) break;
        }
        if (stopped) return;
        setStatus("live");
        if (fresh.length) {
          setEvents((prev) => [...prev, ...fresh].slice(-KEEP));
          if (primed.current) void invalidate();
        }
        if (!primed.current) {
          primed.current = true;
          setReady(true);
        }
      } catch (e) {
        if (errorStatus(e) === 401) redirectToLogin();
        if (!stopped) setStatus("offline");
      }
      if (!stopped) timer = setTimeout(poll, document.hidden ? POLL_MS * 5 : POLL_MS);
    }

    void poll();
    const onVisible = () => {
      if (!document.hidden) {
        clearTimeout(timer);
        void poll();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [invalidate]);

  const value = useMemo(() => ({ events, status, ready }), [events, status, ready]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export const useLive = () => useContext(Ctx);
