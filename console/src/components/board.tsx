import { useMemo, useRef, useState } from "react";
import type { TaskView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { BUCKETS, type Bucket } from "@/lib/format";
import { useIsPhone } from "@/lib/media";
import { Button } from "@/components/ui/button";
import { VirtualColumn } from "@/components/ui/virtual-column";
import { BUCKET_DOT, EmptyState } from "./bits";
import { TaskCard, groupTasks } from "./task-views";

const BOARD_COLS: Bucket[] = ["ready", "claimed", "waiting", "done", "cancelled"];

export function BoardView({
  loading,
  tasks,
  selected,
  onOpen,
  onNew,
  total,
  menus,
  keepOrder = false,
}: {
  loading: boolean;
  tasks: TaskView[];
  selected: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  total: number;
  menus: Record<string, string>;
  keepOrder?: boolean;
}) {
  const groups = useMemo(() => groupTasks(tasks, keepOrder), [tasks, keepOrder]);
  const phone = useIsPhone();
  if (loading) return <BoardSkeleton phone={phone} />;
  if (total === 0) {
    return (
      <EmptyState title="The board is bare" action={<Button onClick={onNew}>New task</Button>}>
        Tasks go from ready to scoop, to on the fire, to served; a task waiting on a person or a dependency sits in between, and tossed ones are kept aside.
      </EmptyState>
    );
  }
  if (phone) return <PhoneBoard groups={groups} selected={selected} onOpen={onOpen} menus={menus} />;
  return (
    <div
      role="region"
      aria-label="Board"
      tabIndex={0}
      className="scrollbar-thin -mx-5 h-[calc(100dvh-16rem)] min-h-96 overflow-x-auto overflow-y-hidden overscroll-x-contain px-5 pb-3 focus-visible:outline-none lg:-mx-8 lg:px-8"
    >
      <div className="flex h-full w-max min-w-full items-stretch gap-3.5">
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          return (
            <VirtualColumn
              key={id}
              aria-label={meta.label}
              className={cn("w-[17.5rem] shrink-0 sm:w-[19rem]", id === "cancelled" && "opacity-80")}
              header={
                <h2 className="flex items-center gap-2 px-4 pt-3.5 pb-2.5 text-[15px]">
                  <span className={cn("size-2.5 rounded-full", BUCKET_DOT[id], id === "claimed" && "live-dot")} aria-hidden="true" />
                  {meta.label}
                  <span className="font-mono text-[11.5px] font-normal text-muted-foreground" style={{ fontFamily: "var(--font-mono)" }}>
                    {col.length}
                  </span>
                </h2>
              }
              items={col}
              itemKey={(t) => t.id}
              empty={<p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">Nothing here.</p>}
              renderItem={(t) => <TaskCard task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />}
            />
          );
        })}
      </div>
    </div>
  );
}


const BOARD_SHORT: Record<Bucket, string> = { ready: "Ready", claimed: "Fire", waiting: "Waiting", done: "Served", cancelled: "Tossed" };
const BOARD_EMPTY: Record<Bucket, string> = {
  ready: "Nothing is waiting to be scooped.",
  claimed: "No cook has anything on the fire.",
  waiting: "Nothing is stuck.",
  done: "Nothing served yet.",
  cancelled: "Nothing tossed.",
};

function BoardSkeleton({ phone }: { phone: boolean }) {
  return (
    <div className="flex gap-3 overflow-hidden" aria-busy="true" aria-label="Loading the board">
      {Array.from({ length: phone ? 1 : 3 }).map((_, c) => (
        <div key={c} className={cn("shrink-0 space-y-2 rounded-[14px] bg-column p-2.5", phone ? "w-full" : "w-[19rem]")}>
          {Array.from({ length: 3 }).map((__, i) => (
            <div key={i} className="space-y-2.5 rounded-[10px] border border-border bg-card p-3">
              <div className="h-2.5 w-12 animate-pulse rounded bg-muted" />
              <div className="h-3.5 animate-pulse rounded bg-muted" style={{ width: `${88 - i * 14}%` }} />
              <div className="h-2 w-2/3 animate-pulse rounded-full bg-muted" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/** Phone board: one column per screen (snap scrolling) and a sticky segmented control that jumps between them. */
function PhoneBoard({
  groups,
  selected,
  onOpen,
  menus,
}: {
  groups: Record<Bucket, TaskView[]>;
  selected: string | null;
  onOpen: (id: string) => void;
  menus: Record<string, string>;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const start = () => {
    const el = scroller.current;
    return el ? parseFloat(getComputedStyle(el).paddingLeft) || 0 : 0;
  };
  const onScroll = () => {
    const el = scroller.current;
    const first = el?.children[0] as HTMLElement | undefined;
    const second = el?.children[1] as HTMLElement | undefined;
    if (!el || !first || !second) return;
    const step = second.offsetLeft - first.offsetLeft;
    setActive(Math.max(0, Math.min(BOARD_COLS.length - 1, Math.round(el.scrollLeft / step))));
  };
  const jump = (i: number) => {
    const el = scroller.current;
    const col = el?.children[i] as HTMLElement | undefined;
    if (!el || !col) return;
    setActive(i);
    el.scrollTo({ left: col.offsetLeft - start(), behavior: "smooth" });
  };
  return (
    <div>
      <div
        role="tablist"
        aria-label="Board columns"
        className="sticky top-[calc(3.5rem+env(safe-area-inset-top))] z-20 -mx-4 bg-background/95 px-4 pb-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/80"
      >
        <div className="grid grid-cols-5 gap-0.5 rounded-xl bg-muted p-1">
          {BOARD_COLS.map((id, i) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={active === i}
              onClick={() => jump(i)}
              className={cn(
                "flex min-h-10 flex-col items-center justify-center rounded-lg px-0.5 text-[11.5px] leading-tight font-medium transition-colors",
                active === i ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              <span className="flex items-center gap-1">
                <span className={cn("size-1.5 rounded-full", BUCKET_DOT[id], id === "claimed" && groups[id].length > 0 && "live-dot")} aria-hidden="true" />
                {BOARD_SHORT[id]}
              </span>
              <span className="font-mono text-[11px] tabular-nums" style={{ fontFamily: "var(--font-mono)" }}>{groups[id].length}</span>
            </button>
          ))}
        </div>
      </div>
      <div
        ref={scroller}
        role="region"
        aria-label="Board"
        onScroll={onScroll}
        className="scrollbar-none relative -mx-4 flex snap-x snap-mandatory items-start gap-3 overflow-x-auto overscroll-x-contain scroll-px-4 px-4 pb-2"
      >
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          return (
            <VirtualColumn
              key={id}
              scroll="window"
              aria-label={`${meta.label}, ${col.length}`}
              className={cn("w-[85vw] shrink-0 snap-start p-2", id === "cancelled" && "opacity-80")}
              header={null}
              items={col}
              itemKey={(t) => t.id}
              empty={<p className="px-2 py-8 text-center text-[13px] text-muted-foreground">{BOARD_EMPTY[id]}</p>}
              renderItem={(t) => <TaskCard task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />}
            />
          );
        })}
      </div>
    </div>
  );
}
