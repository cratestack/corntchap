import { useState } from "react";
import type { TaskView } from "@corntchap/api";
import { ChevronRight, Flame } from "lucide-react";
import { cn } from "@/lib/utils";
import { BUCKETS, bucketOf, fmtLease, leaseLeftMs, taskNote, useNow, type Bucket } from "@/lib/format";
import { useIsPhone } from "@/lib/media";
import { MenuChip } from "./menus";
import { PrChip } from "./forge-prs";
import { ReviewChip } from "./reviews";
import { Avatar, BUCKET_DOT, IdChip, LabelChip, LeaseTimer, ProgressBar, RepoChip } from "./bits";

function priorityTag(p: number) {
  if (p >= 2) return "urgent";
  if (p === 1) return "high";
  if (p < 0) return "low";
  return null;
}

type MenuNames = Record<string, string>;


/** Lease countdown only (saffron, live), for the tight meta lines. The epoch stays in the task detail. */
function MiniLease({ task }: { task: TaskView }) {
  const now = useNow();
  const left = leaseLeftMs(task.lease_expires_at, now);
  if (left === null) return null;
  const expired = left <= 0;
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-1 font-mono text-[11px] tabular-nums", expired ? "text-muted-foreground" : "text-corn-text")} title="Lease time left">
      <span aria-hidden="true" className={cn("size-1.5 rounded-full", expired ? "bg-muted-foreground/50" : "live-dot bg-corn")} />
      {fmtLease(left)}
    </span>
  );
}

/** The owner's small avatar, shown when the owner is not the one holding the task. */
function OwnerMark({ task, className }: { task: TaskView; className?: string }) {
  if (!task.owner || task.owner === task.claimed_by) return null;
  return (
    <span className="inline-flex shrink-0 items-center" title={`Owner: ${task.owner}`} role="img" aria-label={`Owner: ${task.owner}`}>
      <Avatar name={task.owner} className={cn("size-4 rounded-full text-[9px] ring-1 ring-card", className)} />
    </span>
  );
}

function PrioTag({ prio }: { prio: string | null }) {
  if (!prio) return null;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-0.5 rounded px-1.5 py-px font-mono text-[10px] uppercase tracking-wider",
        prio === "low" ? "bg-muted text-muted-foreground" : "bg-primary/15 text-primary",
      )}
    >
      {prio === "urgent" && <Flame className="size-2.5" aria-hidden="true" />}
      {prio}
    </span>
  );
}

/** Two-line row for phones (Linear style): status dot and title, then mono id, holder, progress and labels. */
function PhoneRow({ task, selected, onOpen, menus }: { task: TaskView; selected: boolean; onOpen: (id: string) => void; menus?: MenuNames | undefined }) {
  const bucket = bucketOf(task);
  const menuName = task.menu ? (menus?.[task.menu] ?? undefined) : undefined;
  const done = task.total_items - task.open_items;
  const live = bucket === "claimed";
  return (
    <button
      type="button"
      onClick={() => onOpen(task.id)}
      aria-label={`${task.id}: ${task.title}`}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex min-h-14 w-full items-start gap-3 px-3.5 py-2.5 text-left transition-colors active:bg-muted/70",
        selected && "bg-primary/5",
        task.status === "done" && "opacity-75",
        task.status === "cancelled" && "opacity-60",
      )}
    >
      <span className={cn("mt-[7px] size-2.5 shrink-0 rounded-full", BUCKET_DOT[bucket], live && "live-dot")} aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="flex items-start gap-2">
          <span className={cn("line-clamp-2 min-w-0 flex-1 text-[15px] leading-snug font-medium", task.status === "cancelled" && "line-through")}>{task.title}</span>
          <PrioTag prio={priorityTag(task.priority)} />
        </span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={task.id} className="text-[11.5px] font-normal text-muted-foreground" />
          {task.claimed_by && (
            <span className="inline-flex items-center gap-1">
              <Avatar name={task.claimed_by} className="size-4 rounded-[5px] text-[9px]" />
              <span className="max-w-24 truncate">{task.claimed_by}</span>
            </span>
          )}
          <OwnerMark task={task} />
          {live && <MiniLease task={task} />}
          {task.total_items > 0 && (
            <span className="font-mono text-[11px] tabular-nums">
              {done}/{task.total_items}
            </span>
          )}
          {menuName && <MenuChip name={menuName} />}
          <PrChip pr={task.pr} />
          <ReviewChip review={task.review} />
          {task.repos.map((r) => (
            <RepoChip key={r}>{r}</RepoChip>
          ))}
          {task.labels.slice(0, 2).map((l) => (
            <LabelChip key={l}>{l}</LabelChip>
          ))}
          {task.labels.length > 2 && <span className="font-mono text-[11px]">+{task.labels.length - 2}</span>}
        </span>
      </span>
    </button>
  );
}

export function TaskRow({ task, selected, onOpen, menus }: { task: TaskView; selected: boolean; onOpen: (id: string) => void; menus?: MenuNames | undefined }) {
  const menuName = task.menu ? (menus?.[task.menu] ?? undefined) : undefined;
  const note = taskNote(task);
  const prio = priorityTag(task.priority);
  const live = task.status === "claimed" && task.lease_live;
  const phone = useIsPhone();
  const done = task.total_items - task.open_items;
  if (phone) return <PhoneRow task={task} selected={selected} onOpen={onOpen} menus={menus} />;
  return (
    <button
      type="button"
      onClick={() => onOpen(task.id)}
      aria-label={`${task.id}: ${task.title}`}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "group grid w-full grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 rounded-[10px] border bg-card px-3.5 py-3 text-left transition-colors sm:grid-cols-[4.5rem_minmax(0,1fr)_auto]",
        "hover:border-foreground/25 focus-visible:border-foreground/40",
        selected ? "border-primary ring-1 ring-primary/40" : "border-border",
        task.status === "done" && "opacity-80",
        task.status === "cancelled" && "opacity-60",
      )}
    >
      <IdChip id={task.id} className="pt-px" />
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className={cn("truncate text-[14px] font-medium", task.status === "cancelled" && "line-through")}>{task.title}</span>
          {prio && (
            <span
              className={cn(
                "inline-flex shrink-0 items-center gap-0.5 rounded px-1.5 py-px font-mono text-[10px] uppercase tracking-wider",
                prio === "low" ? "bg-muted text-muted-foreground" : "bg-primary/15 text-primary",
              )}
            >
              {prio === "urgent" && <Flame className="size-2.5" aria-hidden="true" />}
              {prio}
            </span>
          )}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
          {task.total_items > 0 && <ProgressBar done={done} total={task.total_items} />}
          {menuName && <MenuChip name={menuName} />}
          <PrChip pr={task.pr} />
          <ReviewChip review={task.review} />
          {task.repos.map((r) => (
            <RepoChip key={r}>{r}</RepoChip>
          ))}
          {task.labels.map((l) => (
            <LabelChip key={l}>{l}</LabelChip>
          ))}
          {note && <span className="min-w-0 max-w-full truncate text-[12px] text-muted-foreground">{note}</span>}
        </div>
      </div>
      <div className="col-span-2 flex items-center gap-3 sm:col-span-1 sm:justify-end">
        <OwnerMark task={task} className="size-5 text-[10px]" />
        {task.claimed_by && (
          <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <Avatar name={task.claimed_by} className="size-5 text-[10px]" />
            <span className="max-w-24 truncate">{task.claimed_by}</span>
          </span>
        )}
        {live && <LeaseTimer task={task} />}
        <ChevronRight className="hidden size-4 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 sm:block" aria-hidden="true" />
      </div>
    </button>
  );
}

export function TaskGroup({
  bucket,
  tasks,
  selected,
  onOpen,
  menus,
  defaultOpen = true,
}: {
  bucket: Bucket;
  tasks: TaskView[];
  selected: string | null;
  onOpen: (id: string) => void;
  menus?: MenuNames | undefined;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const [all, setAll] = useState(false);
  const phone = useIsPhone();
  const meta = BUCKETS.find((b) => b.id === bucket)!;
  const cap = bucket === "done" || bucket === "cancelled" ? 8 : tasks.length;
  const shown = all ? tasks : tasks.slice(0, cap);
  return (
    <section aria-labelledby={`g-${bucket}`}>
      <h2 id={`g-${bucket}`} className="m-0 text-base">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
          className="flex min-h-11 w-full items-center gap-2 rounded-md py-1 text-left md:min-h-0"
        >
          <ChevronRight className={cn("size-4 text-muted-foreground transition-transform", open && "rotate-90")} aria-hidden="true" />
          <span className={cn("size-2.5 rounded-full", BUCKET_DOT[bucket], bucket === "claimed" && "live-dot")} aria-hidden="true" />
          <span className="font-display text-[16px]">{meta.label}</span>
          <span className="font-mono text-[11.5px] text-muted-foreground">{tasks.length}</span>
          <span className="ml-1 hidden text-[12px] font-normal text-muted-foreground sm:inline" style={{ fontFamily: "var(--font-sans)", letterSpacing: 0 }}>
            {meta.hint}
          </span>
        </button>
      </h2>
      {open && (
        <div className={cn("mt-1.5", phone ? "divide-y divide-border overflow-hidden rounded-xl border border-border bg-card" : "space-y-1.5")}>
          {shown.map((t) => (
            <TaskRow key={t.id} task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />
          ))}
          {!all && tasks.length > shown.length && (
            <button type="button" onClick={() => setAll(true)} className="min-h-11 w-full px-3.5 py-1 text-left text-[13px] text-primary hover:underline md:min-h-0 md:w-auto md:px-2 md:text-[12.5px]">
              Show {tasks.length - shown.length} more
            </button>
          )}
        </div>
      )}
    </section>
  );
}

export function TaskCard({ task, selected, onOpen, menus }: { task: TaskView; selected: boolean; onOpen: (id: string) => void; menus?: MenuNames | undefined }) {
  const menuName = task.menu ? (menus?.[task.menu] ?? undefined) : undefined;
  const live = task.status === "claimed" && task.lease_live;
  const done = task.total_items - task.open_items;
  const note = taskNote(task);
  const prio = priorityTag(task.priority);
  const extra = task.labels.length - 2;
  return (
    <button
      type="button"
      onClick={() => onOpen(task.id)}
      aria-label={`${task.id}: ${task.title}`}
      className={cn(
        "block w-full rounded-[10px] border bg-card px-3 py-2.5 text-left shadow-[0_1px_0_0_color-mix(in_oklab,var(--ink)_6%,transparent)] transition-colors hover:border-foreground/25 active:bg-muted/50",
        selected ? "border-primary ring-1 ring-primary/40" : "border-border",
        task.status === "cancelled" && "opacity-70",
      )}
    >
      <div className="flex items-center gap-2">
        <IdChip id={task.id} className="text-[11.5px]" />
        <PrioTag prio={prio} />
        {menuName && <MenuChip name={menuName} />}
      </div>
      <p className={cn("mt-1 line-clamp-3 text-[14px] leading-snug font-medium", task.status === "cancelled" && "line-through")}>{task.title}</p>
      {note && <p className="mt-1 line-clamp-1 text-[12px] text-muted-foreground">{note}</p>}
      {(task.repos.length > 0 || task.pr || task.review) && (
        <div className="mt-1.5 flex flex-wrap items-center gap-1">
          <PrChip pr={task.pr} />
          <ReviewChip review={task.review} />
          {task.repos.map((r) => (
            <RepoChip key={r}>{r}</RepoChip>
          ))}
        </div>
      )}
      {(task.total_items > 0 || task.labels.length > 0 || task.claimed_by || task.owner || live) && (
        <div className="mt-2 flex items-center gap-2 overflow-hidden">
          {task.total_items > 0 && <ProgressBar done={done} total={task.total_items} className="shrink-0 [&>div:first-child]:w-12" />}
          <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden">
            {task.labels.slice(0, 2).map((l) => (
              <LabelChip key={l} className="max-w-24 shrink-0 truncate">{l}</LabelChip>
            ))}
            {extra > 0 && <span className="shrink-0 font-mono text-[11px] text-muted-foreground">+{extra}</span>}
          </div>
          {live && <MiniLease task={task} />}
          <OwnerMark task={task} className="size-5 text-[10px]" />
          {task.claimed_by && <Avatar name={task.claimed_by} className="size-5 text-[10px]" />}
        </div>
      )}
    </button>
  );
}

/** Bucket the tasks; `keepOrder` leaves them as given (a saved view's own order) instead of best-first. */
export function groupTasks(tasks: TaskView[], keepOrder = false): Record<Bucket, TaskView[]> {
  const g: Record<Bucket, TaskView[]> = { claimed: [], ready: [], waiting: [], done: [], cancelled: [] };
  for (const t of tasks) g[bucketOf(t)].push(t);
  if (keepOrder) return g;
  // best first: priority, then oldest; served: most recent first
  const best = (a: TaskView, b: TaskView) => b.priority - a.priority || a.number - b.number;
  g.claimed.sort(best);
  g.ready.sort(best);
  g.waiting.sort(best);
  g.done.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  g.cancelled.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  return g;
}
