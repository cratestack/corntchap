// Generated data and tiny live stores for the virtual list stories; not used by the app.
import { useMemo, useSyncExternalStore, type ReactNode } from "react";
import type { ChangeView, TaskView } from "@corntchap/api";
import { staticSource, type ListSource } from "@/lib/listbox";

export interface FixtureRow {
  id: number;
  title: string;
  /** Between 1 and 4 lines, so row heights are mixed. */
  lines: number;
}

export function makeRows(count: number, from = 0): FixtureRow[] {
  return Array.from({ length: count }, (_, i) => ({ id: from + i + 1, title: `Row ${from + i + 1}`, lines: 1 + ((from + i) % 4) }));
}

export function RowCard({ row }: { row: FixtureRow }) {
  return (
    <div data-testid="row" className="border-b border-border bg-card px-4 py-3">
      <p className="font-mono text-[12px] font-bold">{row.title}</p>
      {Array.from({ length: row.lines }, (_, i) => (
        <p key={i} className="text-[13px] text-muted-foreground">
          Line {i + 1} of {row.lines}: a few words about this row so that it has some height.
        </p>
      ))}
    </div>
  );
}

const KINDS = ["task.created", "task.claimed", "item.checked", "task.commented", "task.completed", "task.released", "memory.saved"];
const ACTORS = ["ada", "bola", "chidi", "ngozi", "kemi"];
const NOW = Date.now();

/** Oldest first, like `useLive().events`: one every 7 minutes back from now, so a 10k feed spans about 7 weeks of days. */
export function makeEvents(count: number, from = 0): ChangeView[] {
  return Array.from({ length: count }, (_, i) => {
    const n = from + i;
    return {
      seq: n + 1,
      kind: KINDS[n % KINDS.length]!,
      project: "PAY",
      task: `PAY-${(n % 480) + 1}`,
      actor: ACTORS[n % ACTORS.length]!,
      at: new Date(NOW - (from + count - n) * 7 * 60_000).toISOString(),
      payload: { title: `Event ${n + 1}: ${"a longer title ".repeat(n % 3)}`.trim(), result: n % 5 === 0 ? "all green, merged and deployed" : undefined },
    } as ChangeView;
  });
}

const STATUSES = ["open", "claimed", "blocked", "done", "cancelled"];

export function makeTasks(count: number): TaskView[] {
  return Array.from({ length: count }, (_, i) => {
    const status = STATUSES[i % STATUSES.length]!;
    return {
      id: `PAY-${i + 1}`,
      project: "PAY",
      number: i + 1,
      title: `Task ${i + 1}: ${"wire the thing through the other thing ".repeat(1 + (i % 3))}`.trim(),
      body: "",
      priority: (i % 5) - 1,
      labels: i % 3 === 0 ? ["backend", "payments", "urgent"] : i % 3 === 1 ? ["web"] : [],
      repos: i % 4 === 0 ? ["acme/shop"] : [],
      status,
      depends_on: [],
      claimed_by: status === "claimed" ? ACTORS[i % ACTORS.length]! : null,
      owner: null,
      lease_epoch: 1,
      lease_expires_at: status === "claimed" ? new Date(NOW + 20 * 60_000).toISOString() : null,
      lease_live: status === "claimed",
      ready: status === "open",
      open_items: i % 4,
      total_items: i % 6,
      menu: null,
      created_at: new Date(NOW - i * 60_000).toISOString(),
      updated_at: new Date(NOW - i * 60_000).toISOString(),
      pr: null,
      review: null,
    } as TaskView;
  });
}

/** Grows by `step` items every `ms`, until `max`; a stand-in for the live event stream. */
export function useGrowing<T>(make: (count: number) => T[], start: number, step = 1, ms = 1000, max = 400): T[] {
  const store = useMemo(() => {
    let count = start;
    let items = make(count);
    const listeners = new Set<() => void>();
    let timer: ReturnType<typeof setInterval> | undefined;
    return {
      get: () => items,
      subscribe: (l: () => void) => {
        listeners.add(l);
        timer ??= setInterval(() => {
          count = Math.min(max, count + step);
          items = make(count);
          listeners.forEach((f) => f());
        }, ms);
        return () => {
          listeners.delete(l);
          if (listeners.size === 0) {
            clearInterval(timer);
            timer = undefined;
          }
        };
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return useSyncExternalStore(store.subscribe, store.get);
}

/**
 * Tasks of one board column (`bucket`), numbered `1..count` in the column's own order: what `taskPage` answers
 * for it. Every column has its own run of ids, so the columns can all be 10,000 long.
 */
export function makeColumnTasks(bucket: "ready" | "claimed" | "waiting" | "done" | "cancelled", count: number): TaskView[] {
  const status = { ready: "open", claimed: "claimed", waiting: "blocked", done: "done", cancelled: "cancelled" }[bucket];
  return makeTasks(count).map((t, i) => ({
    ...t,
    status,
    claimed_by: bucket === "claimed" ? ACTORS[i % ACTORS.length]! : null,
    lease_live: bucket === "claimed",
    lease_expires_at: bucket === "claimed" ? new Date(NOW + 20 * 60_000).toISOString() : null,
    ready: bucket === "ready",
  }));
}

/**
 * Pages in `page` more items `ms` after `loadMore()`, up to `total`; a stand-in for a server-paged bucket.
 * `loads` counts the pages asked for, so a story can show which columns loaded.
 */
export function usePaged<T>(make: (count: number) => T[], first: number, page: number, total: number, ms = 600) {
  const store = useMemo(() => {
    let state = { items: make(first), loading: false, loads: 0 };
    const listeners = new Set<() => void>();
    const set = (next: typeof state) => {
      state = next;
      listeners.forEach((f) => f());
    };
    return {
      get: () => state,
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
      loadMore: () => {
        if (state.loading || state.items.length >= total) return;
        set({ ...state, loading: true, loads: state.loads + 1 });
        setTimeout(() => set({ ...state, items: make(Math.min(total, state.items.length + page)), loading: false }), ms);
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const { items, loading, loads } = useSyncExternalStore(store.subscribe, store.get);
  return { items, loading, loads, done: items.length >= total, loadMore: store.loadMore };
}

// ---- a chat thread: oldest first, older pages load on request, new messages can arrive ----

export interface ThreadMsg {
  id: string;
  seq: number;
  /** Between 1 and 5 lines, so heights are mixed. */
  lines: number;
  mine: boolean;
}

/** Messages `from + 1 .. from + count`, oldest first. */
export function makeThread(count: number, from = 0): ThreadMsg[] {
  return Array.from({ length: count }, (_, i) => {
    const seq = from + i + 1;
    return { id: `m${seq}`, seq, lines: 1 + ((seq * 7) % 5), mine: seq % 3 === 0 };
  });
}

export function MsgBubble({ msg }: { msg: ThreadMsg }) {
  return (
    <div data-testid="msg" data-seq={msg.seq} className={msg.mine ? "flex justify-end pt-1" : "flex justify-start pt-1"}>
      <div className={`max-w-[75%] rounded-2xl px-3.5 py-2 ${msg.mine ? "bg-primary/15" : "bg-muted"}`}>
        <p className="font-mono text-[11px] leading-4 font-bold">Message {msg.seq}</p>
        {Array.from({ length: msg.lines }, (_, i) => (
          <p key={i} className="text-[13px] leading-5">
            Line {i + 1} of {msg.lines}: what somebody said in the thread.
          </p>
        ))}
      </div>
    </div>
  );
}

/**
 * A server-paged thread of `total` messages: the newest `loaded` first, `loadOlder()` prepends `olderPage` more
 * after `olderMs`, and with `appendMs` a new message arrives at that interval (until `maxAppended`).
 */
export function useThread({ total, loaded, olderPage = 50, olderMs = 300, appendMs = 0, maxAppended = 50 }: { total: number; loaded: number; olderPage?: number; olderMs?: number; appendMs?: number; maxAppended?: number }) {
  const store = useMemo(() => {
    let state = { items: makeThread(loaded, total - loaded), loadingOlder: false };
    const listeners = new Set<() => void>();
    const set = (next: typeof state) => {
      state = next;
      listeners.forEach((f) => f());
    };
    let appended = 0;
    let timer: ReturnType<typeof setInterval> | undefined;
    return {
      get: () => state,
      subscribe: (l: () => void) => {
        listeners.add(l);
        if (appendMs > 0 && timer === undefined) {
          timer = setInterval(() => {
            if (appended >= maxAppended) return;
            appended += 1;
            set({ ...state, items: [...state.items, ...makeThread(1, total + appended - 1)] });
          }, appendMs);
        }
        return () => {
          listeners.delete(l);
          if (listeners.size === 0) {
            clearInterval(timer);
            timer = undefined;
          }
        };
      },
      loadOlder: () => {
        const first = state.items[0]?.seq ?? 1;
        if (state.loadingOlder || first <= 1) return;
        set({ ...state, loadingOlder: true });
        setTimeout(() => {
          const take = Math.min(olderPage, first - 1);
          set({ items: [...makeThread(take, first - 1 - take), ...state.items], loadingOlder: false });
        }, olderMs);
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const state = useSyncExternalStore(store.subscribe, store.get);
  const first = state.items[0]?.seq ?? 1;
  return { items: state.items, loadingOlder: state.loadingOlder, hasOlder: first > 1, loadOlder: store.loadOlder, lastSeq: state.items[state.items.length - 1]?.seq ?? 0 };
}

// ---- pickers: people and labels, and a source that pages and searches like the server ----

export interface FixturePerson {
  id: string;
  /** `cook-1` .. `cook-N` (every fifth is a person, not an agent). */
  name: string;
  display: string;
  kind: "agent" | "user";
}

export function makePeople(count: number): FixturePerson[] {
  return Array.from({ length: count }, (_, i) => ({
    id: `cook-${i + 1}`,
    name: `cook-${i + 1}`,
    display: `Cook number ${i + 1}`,
    kind: (i + 1) % 5 === 0 ? "user" : "agent",
  }));
}

export function makeLabels(count: number): string[] {
  return Array.from({ length: count }, (_, i) => `label-${i + 1}`);
}

export const matchPerson = (p: FixturePerson, q: string) => `${p.name} ${p.display}`.toLowerCase().includes(q.trim().toLowerCase());
export const matchString = (s: string, q: string) => s.toLowerCase().includes(q.trim().toLowerCase());

export function PersonRow({ person }: { person: FixturePerson }) {
  return (
    <span className="flex min-w-0 items-baseline gap-2">
      <span className="truncate font-mono text-[13px] font-medium">{person.name}</span>
      <span className="truncate text-[12px] text-muted-foreground">{person.display}</span>
    </span>
  );
}

/** A source that has not answered yet (the first page is loading). */
export function loadingSource<T>(): ListSource<T> {
  return { ...staticSource<T>([]), isLoading: true, total: null };
}

/** A source whose first page failed. */
export function errorSource<T>(): ListSource<T> {
  return { ...staticSource<T>([]), error: new Error("The server did not answer. Try again."), total: null };
}

/**
 * Pages and searches `all` like a server would: `pageSize` rows at a time (`latencyMs` after the request), narrowed by
 * `match` as `onQuery` is told the search text. `loads` counts the pages asked for.
 */
export function usePagedSource<T>(all: readonly T[], opts: { pageSize?: number; latencyMs?: number; match: (item: T, q: string) => boolean; /** The search text already in force (from the URL). */ initialQuery?: string }) {
  const pageSize = opts.pageSize ?? 25;
  const latencyMs = opts.latencyMs ?? 250;
  const store = useMemo(() => {
    let query = opts.initialQuery ?? "";
    let applied = query;
    let shown = pageSize;
    let loading = false;
    let loads = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const listeners = new Set<() => void>();
    const filtered = (q: string) => (q.trim() ? all.filter((it) => opts.match(it, q)) : all);
    let rows = filtered(query);
    const snapshot = () => {
      const source: ListSource<T> = {
        items: rows.slice(0, shown),
        hasNextPage: shown < rows.length,
        fetchNextPage: more,
        isLoading: false,
        isFetchingNextPage: loading && applied === query,
        error: null,
        total: rows.length,
      };
      return { source, query, loads };
    };
    let current = snapshot();
    const publish = () => {
      current = snapshot();
      listeners.forEach((l) => l());
    };
    function more() {
      if (loading || shown >= rows.length) return;
      loading = true;
      loads += 1;
      publish();
      setTimeout(() => {
        shown += pageSize;
        loading = false;
        publish();
      }, latencyMs);
    }
    return {
      get: () => current,
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
      onQuery: (q: string) => {
        query = q;
        clearTimeout(timer);
        timer = setTimeout(() => {
          applied = q;
          rows = filtered(q);
          shown = pageSize;
          loading = false;
          publish();
        }, latencyMs);
        publish();
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [all, pageSize, latencyMs]);
  const { source, query, loads } = useSyncExternalStore(store.subscribe, store.get);
  return { source, onQuery: store.onQuery, query, loads };
}

/** What the story's picker last chose, for a test (or a person) to read. */
export function PickedLine({ value }: { value: string }) {
  return (
    <p className="mt-3 text-[12.5px] text-muted-foreground">
      Picked: <span data-testid="picked" className="font-mono text-foreground">{value}</span>
    </p>
  );
}

/** A page for a popover story to sit on. */
export function StoryPage({ children }: { children: ReactNode }) {
  return <div className="min-h-dvh bg-background p-4 md:p-8">{children}</div>;
}
