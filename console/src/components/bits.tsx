import type { ReactNode } from "react";
import type { TaskView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { colorFor, fmtLease, initial, leaseLeftMs, useNow, type Bucket } from "@/lib/format";
import { WaxBand } from "./brand";

export function Avatar({ name, className, title }: { name: string; className?: string; title?: string }) {
  return (
    <span
      title={title ?? name}
      aria-hidden="true"
      className={cn(
        "inline-grid size-6 shrink-0 place-items-center rounded-md font-display text-[12px] text-white",
        className,
      )}
      style={{ background: colorFor(name) }}
    >
      {initial(name)}
    </span>
  );
}

export type CookState = "cooking" | "awake" | "idle" | "disabled";

/** Saffron + pulsing = live. Everything else is quiet. */
export function StateDot({ state, className }: { state: CookState; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block size-2 shrink-0 rounded-full",
        state === "cooking" && "live-dot bg-corn",
        state === "awake" && "bg-corn",
        state === "idle" && "bg-sidebar-muted/50",
        state === "disabled" && "bg-destructive",
        className,
      )}
    />
  );
}

export const STATE_LABEL: Record<CookState, string> = {
  cooking: "cooking",
  awake: "awake",
  idle: "idle",
  disabled: "disabled",
};

export function cookState(a: { disabled: boolean; busy: number; last_seen_at: string | null }, now: number): CookState {
  if (a.disabled) return "disabled";
  if (a.busy > 0) return "cooking";
  if (a.last_seen_at && now - new Date(a.last_seen_at).getTime() < 5 * 60_000) return "awake";
  return "idle";
}

export function ProgressBar({ done, total, className }: { done: number; total: number; className?: string }) {
  const pct = total === 0 ? 0 : Math.round((done / total) * 100);
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div
        role="progressbar"
        aria-label="Checklist progress"
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={done}
        className="h-1.5 w-20 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn("h-full rounded-full transition-[width] duration-500", done === total && total > 0 ? "bg-success" : "bg-primary")}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
        {done}/{total}
      </span>
    </div>
  );
}

export function LabelChip({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full border border-border bg-muted/60 px-2 py-px text-[11px] font-medium text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** Lease countdown (saffron) and the fencing epoch, ticking client-side from the server's expiry. */
export function LeaseTimer({ task, className }: { task: Pick<TaskView, "lease_expires_at" | "lease_epoch" | "lease_live">; className?: string }) {
  const now = useNow();
  const left = leaseLeftMs(task.lease_expires_at, now);
  const expired = left !== null && left <= 0;
  return (
    <span className={cn("inline-flex items-center gap-2 font-mono text-[11px] tabular-nums", className)}>
      {left !== null && (
        <span
          className={cn("inline-flex items-center gap-1.5", expired ? "text-muted-foreground" : "text-corn-text")}
          title="Lease time left"
        >
          <span aria-hidden="true" className={cn("size-1.5 rounded-full", expired ? "bg-muted-foreground/50" : "live-dot bg-corn")} />
          {fmtLease(left)}
        </span>
      )}
      <span className="text-muted-foreground" title="Fencing epoch: bumps on every claim">
        epoch {task.lease_epoch}
      </span>
    </span>
  );
}

export function IdChip({ id, className }: { id: string; className?: string }) {
  return <span className={cn("font-mono text-[12px] font-bold tracking-tight text-foreground/80", className)}>{id}</span>;
}

export const BUCKET_DOT: Record<Bucket, string> = {
  claimed: "bg-primary",
  ready: "bg-corn",
  waiting: "bg-waiting",
  done: "bg-success",
  cancelled: "bg-muted-foreground/50",
};

export function EmptyState({
  title,
  children,
  action,
  band = true,
  className,
}: {
  title: string;
  children?: ReactNode;
  action?: ReactNode;
  band?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("overflow-hidden rounded-2xl border border-border bg-card", className)}>
      {band && <WaxBand height={52} />}
      <div className="px-6 py-8 text-center">
        <h3 className="text-xl">{title}</h3>
        {children && <p className="mx-auto mt-1.5 max-w-md text-muted-foreground">{children}</p>}
        {action && <div className="mt-5 flex justify-center">{action}</div>}
      </div>
    </div>
  );
}

export function SectionLabel({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("label-caps", className)}>{children}</div>;
}

export function RowSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div className="space-y-2" aria-busy="true" aria-label="Loading">
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="flex h-[58px] items-center gap-4 rounded-lg border border-border bg-card px-4">
          <div className="h-3 w-14 animate-pulse rounded bg-muted" />
          <div className="h-3 flex-1 animate-pulse rounded bg-muted" style={{ maxWidth: `${60 - i * 8}%` }} />
          <div className="hidden h-2 w-20 animate-pulse rounded-full bg-muted sm:block" />
        </div>
      ))}
    </div>
  );
}
