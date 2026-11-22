import { useState } from "react";
import type { TaskView } from "@corntchap/api";
import { ChevronRight, Flame } from "lucide-react";
import { cn } from "@/lib/utils";
import { BUCKETS, bucketOf, taskNote, type Bucket } from "@/lib/format";
import { MenuChip } from "./menus";
import { Avatar, BUCKET_DOT, IdChip, LabelChip, LeaseTimer, ProgressBar } from "./bits";

function priorityTag(p: number) {
  if (p >= 2) return "urgent";
  if (p === 1) return "high";
  if (p < 0) return "low";
  return null;
}

type MenuNames = Record<string, string>;

export function TaskRow({ task, selected, onOpen, menus }: { task: TaskView; selected: boolean; onOpen: (id: string) => void; menus?: MenuNames | undefined }) {
  const menuName = task.menu ? (menus?.[task.menu] ?? undefined) : undefined;
  const note = taskNote(task);
  const prio = priorityTag(task.priority);
  const live = task.status === "claimed" && task.lease_live;
  const done = task.total_items - task.open_items;
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
          {task.labels.map((l) => (
            <LabelChip key={l}>{l}</LabelChip>
          ))}
          {note && <span className="min-w-0 max-w-full truncate text-[12px] text-muted-foreground">{note}</span>}
        </div>
      </div>
      <div className="col-span-2 flex items-center gap-3 sm:col-span-1 sm:justify-end">
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
          className="flex w-full items-center gap-2 rounded-md py-1 text-left"
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
        <div className="mt-1.5 space-y-1.5">
          {shown.map((t) => (
            <TaskRow key={t.id} task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />
          ))}
          {!all && tasks.length > shown.length && (
            <button type="button" onClick={() => setAll(true)} className="px-2 py-1 text-[12.5px] text-primary hover:underline">
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
  return (
    <button
      type="button"
      onClick={() => onOpen(task.id)}
      aria-label={`${task.id}: ${task.title}`}
      className={cn(
        "w-full space-y-2 rounded-[10px] border bg-card p-3 text-left shadow-[0_1px_0_0_color-mix(in_oklab,var(--ink)_6%,transparent)] transition-colors hover:border-foreground/25",
        selected ? "border-primary ring-1 ring-primary/40" : "border-border",
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <IdChip id={task.id} />
        {live && <LeaseTimer task={task} />}
      </div>
      <p className="line-clamp-3 text-[13.5px] font-medium leading-snug">{task.title}</p>
      {note && <p className="line-clamp-2 text-[12px] text-muted-foreground">{note}</p>}
      {task.total_items > 0 && <ProgressBar done={done} total={task.total_items} />}
      {(task.labels.length > 0 || task.claimed_by || menuName) && (
        <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
          {menuName && <MenuChip name={menuName} />}
          {task.labels.map((l) => (
            <LabelChip key={l}>{l}</LabelChip>
          ))}
          {task.claimed_by && (
            <span className="ml-auto inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
              <Avatar name={task.claimed_by} className="size-5 text-[10px]" />
              {task.claimed_by}
            </span>
          )}
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
