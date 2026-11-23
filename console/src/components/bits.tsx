import type { ReactNode } from "react";
import { Link } from "react-router";
import { ChevronRight } from "lucide-react";
import type { TaskView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { colorFor, fmtLease, initial, leaseLeftMs, useNow, type Bucket } from "@/lib/format";
import { WaxBand } from "./brand";

/** A person as humans read it: "Display Name · @handle"; without a display name (cooks, fresh accounts) just the name. */
export function PersonName({ name, displayName, className }: { name: string; displayName?: string | null | undefined; className?: string }) {
  if (!displayName) return <span className={cn("font-medium", className)}>{name}</span>;
  return (
    <span className={cn("min-w-0", className)}>
      <span className="font-medium">{displayName}</span>
      <span className="text-muted-foreground"> · </span>
      <span className="font-mono text-[12px] text-muted-foreground">@{name}</span>
    </span>
  );
}

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

/** A repository alias on a task or a cook scope: mono, with a primary tint so it does not read as a label. */
export function RepoChip({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full border border-primary/30 bg-primary/5 px-2 py-px font-mono text-[11px] font-medium text-foreground",
        className,
      )}
    >
      {children}
    </span>
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

// ---------------------------------------------------------------- settings-style lists (phone first)

/** A titled, grouped list: mono caps label above, one rounded card of rows below (Linear-style settings). */
export function Group({
  title,
  hint,
  aside,
  children,
  className,
  tone,
}: {
  title: string;
  hint?: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
  tone?: "danger";
}) {
  return (
    <section aria-label={title} className={cn("min-w-0", className)}>
      <div className="mb-1.5 flex items-baseline justify-between gap-3 px-1">
        <h2 className={cn("label-caps", tone === "danger" && "text-destructive")}>{title}</h2>
        {aside}
      </div>
      <div
        className={cn(
          "divide-y divide-border overflow-hidden rounded-[14px] border bg-card",
          tone === "danger" ? "border-destructive/30 divide-destructive/15" : "border-border",
        )}
      >
        {children}
      </div>
      {hint && <p className="mt-1.5 px-1 text-[12px] leading-snug text-muted-foreground">{hint}</p>}
    </section>
  );
}

/**
 * A row of a Group, at least 48px: icon, label (with an optional second line), a value and a chevron.
 * With `to` it is a link, with `onClick` a button, otherwise plain.
 */
export function GroupRow({
  icon,
  label,
  hint,
  value,
  trailing,
  to,
  onClick,
  chevron,
  destructive,
  disabled,
  className,
}: {
  icon?: ReactNode;
  label: ReactNode;
  hint?: ReactNode;
  value?: ReactNode;
  trailing?: ReactNode;
  to?: string;
  onClick?: () => void;
  chevron?: boolean;
  destructive?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const interactive = !!to || !!onClick;
  const cls = cn(
    "flex min-h-12 w-full min-w-0 items-center gap-3 px-4 py-2.5 text-left",
    interactive && "outline-none transition-colors hover:bg-muted/50 focus-visible:bg-muted/60 active:bg-muted disabled:opacity-50",
    destructive && "text-destructive",
    className,
  );
  const body = (
    <>
      {icon && (
        <span
          aria-hidden="true"
          className={cn("grid size-8 shrink-0 place-items-center rounded-lg [&_svg]:size-4", destructive ? "bg-destructive/10" : "bg-muted text-muted-foreground")}
        >
          {icon}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className={cn("block text-[14px] font-medium leading-tight", !destructive && "text-foreground")}>{label}</span>
        {hint && <span className="mt-0.5 block text-[12px] leading-snug font-normal text-muted-foreground">{hint}</span>}
      </span>
      {value !== undefined && value !== null && <span className="ml-auto min-w-0 max-w-[55%] truncate text-right text-[13px] text-muted-foreground">{value}</span>}
      {trailing}
      {interactive && chevron !== false && <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />}
    </>
  );
  if (to) {
    return (
      <Link to={to} className={cls}>
        {body}
      </Link>
    );
  }
  if (onClick) {
    return (
      <button type="button" onClick={onClick} disabled={disabled} className={cls}>
        {body}
      </button>
    );
  }
  return <div className={cls}>{body}</div>;
}

/** Filter chips for a list: one is on; each shows its count in mono. 40px tall on touch. */
export function FilterChips<T extends string>({
  options,
  value,
  onChange,
  label,
  className,
}: {
  options: { id: T; label: string; count?: number }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
  className?: string;
}) {
  return (
    <div role="group" aria-label={label} className={cn("flex flex-wrap gap-1.5", className)}>
      {options.map((o) => {
        const on = o.id === value;
        return (
          <button
            key={o.id}
            type="button"
            aria-pressed={on}
            onClick={() => onChange(o.id)}
            className={cn(
              "inline-flex h-9 items-center gap-1.5 rounded-full border px-3 text-[13px] font-medium transition-colors pointer-coarse:h-10",
              on ? "border-foreground bg-foreground text-background" : "border-border bg-card text-muted-foreground hover:text-foreground",
            )}
          >
            {o.label}
            {o.count !== undefined && <span className={cn("font-mono text-[11px] tabular-nums", on ? "opacity-70" : "opacity-60")}>{o.count}</span>}
          </button>
        );
      })}
    </div>
  );
}

/** A cook's state as a pill: saffron only while it is live. */
export function StatePill({ state, label, className }: { state: CookState; label?: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11.5px] font-medium",
        state === "cooking" || state === "awake" ? "border-corn/60 bg-corn/15" : "border-border bg-muted/60 text-muted-foreground",
        state === "disabled" && "border-destructive/40 text-destructive",
        className,
      )}
    >
      <StateDot state={state} className={state === "idle" ? "bg-muted-foreground/50" : ""} />
      {label ?? STATE_LABEL[state]}
    </span>
  );
}

/** The parts of a bottom sheet on phones (and a plain dialog above): a fixed head, a body that scrolls, a sticky footer. */
export const formSheetClass = "flex max-h-[92dvh] flex-col gap-0 overflow-hidden p-0 max-sm:overflow-hidden";
export function FormSheetHead({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-col gap-2 px-6 pt-6 pb-3 pr-12 text-left max-sm:pt-7", className)}>{children}</div>;
}
export function FormSheetBody({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("min-h-0 flex-1 space-y-4 overflow-y-auto px-6 py-2", className)}>{children}</div>;
}
export function FormSheetFoot({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "flex flex-col gap-2 border-t border-border bg-card px-6 pt-3 pb-[calc(0.875rem+env(safe-area-inset-bottom))] sm:flex-row-reverse sm:justify-start sm:pb-4",
        className,
      )}
    >
      {children}
    </div>
  );
}
