import { useSyncExternalStore } from "react";
import type { TaskView } from "@corntchap/api";

// One shared 1 s clock: every lease timer and "2m ago" label reads the same tick.
let now = Date.now();
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;

function subscribe(cb: () => void) {
  listeners.add(cb);
  if (!timer) {
    timer = setInterval(() => {
      now = Date.now();
      listeners.forEach((l) => l());
    }, 1000);
  }
  return () => {
    listeners.delete(cb);
    if (listeners.size === 0 && timer) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

/** Milliseconds since epoch, re-rendering the caller every second. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, () => now);
}

export function leaseLeftMs(expiresAt: string | null | undefined, at: number): number | null {
  if (!expiresAt) return null;
  return new Date(expiresAt).getTime() - at;
}

/** 4:07, 1h 04m, expired */
export function fmtLease(ms: number | null): string {
  if (ms === null) return "";
  if (ms <= 0) return "expired";
  const s = Math.ceil(ms / 1000);
  if (s < 3600) return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  const h = Math.floor(s / 3600);
  return `${h}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}

export function fmtAgo(iso: string | null | undefined, at: number): string {
  if (!iso) return "never";
  const s = Math.max(0, Math.round((at - new Date(iso).getTime()) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 14) return `${Math.floor(s / 86400)}d ago`;
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function fmtClock(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

// ---- statuses (docs/design.md, "Vocabulary") ----

export type Bucket = "claimed" | "ready" | "waiting" | "done" | "cancelled";

export const BUCKETS: { id: Bucket; label: string; hint: string }[] = [
  { id: "claimed", label: "On the fire", hint: "claimed, lease ticking" },
  { id: "ready", label: "Ready to scoop", hint: "open, nothing in the way" },
  { id: "waiting", label: "Waiting", hint: "blocked or waiting on other tasks" },
  { id: "done", label: "Served", hint: "completed" },
  { id: "cancelled", label: "Cancelled", hint: "cancelled by a human" },
];

export const bucketLabel = (b: Bucket) => BUCKETS.find((x) => x.id === b)!.label;

export function bucketOf(t: Pick<TaskView, "status" | "ready" | "lease_live">): Bucket {
  switch (t.status) {
    case "claimed":
      // an expired lease frees the task for the next cook
      return t.lease_live ? "claimed" : "ready";
    case "open":
      return t.ready ? "ready" : "waiting";
    case "blocked":
      return "waiting";
    case "done":
      return "done";
    default:
      return "cancelled";
  }
}

/** The one-line note on a task row: dependencies, why it is blocked, or the result. */
export function taskNote(t: TaskView): string | null {
  if (t.status === "blocked") return t.result ? `blocked: ${t.result}` : "blocked, waiting for a human to unblock";
  if (t.status === "open" && !t.ready) {
    return t.depends_on.length ? `waits on ${t.depends_on.join(", ")}` : "waits on its subtasks";
  }
  if (t.status === "claimed" && !t.lease_live) return "lease ran out, up for scooping again";
  if (t.status === "done" && t.result) return t.result;
  if (t.status === "cancelled") return t.result ?? "cancelled";
  if (t.status === "open" && t.result) return `released: ${t.result}`;
  return null;
}

export function initial(name: string): string {
  return (name.trim()[0] ?? "?").toUpperCase();
}

const PALETTE = ["#c8502c", "#1f7a72", "#b67a00", "#3d4590", "#8a3b66", "#4d6b2c"];
export function colorFor(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PALETTE[h % PALETTE.length]!;
}

export function plural(n: number, one: string, many = one + "s"): string {
  return `${n} ${n === 1 ? one : many}`;
}
