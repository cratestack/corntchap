import { useLayoutEffect, useMemo, useRef } from "react";
import { useSearchParams } from "react-router";
import type { TaskView } from "@corntchap/api";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { BucketMore } from "@/lib/board";
import { BUCKETS, type Bucket } from "@/lib/format";
import { useFillTop } from "@/lib/use-fill-height";
import { Button } from "@/components/ui/button";
import { VirtualColumn } from "@/components/ui/virtual-column";
import { BUCKET_DOT, EmptyState } from "./bits";
import { TaskCard, groupTasks } from "./task-views";

const BOARD_COLS: Bucket[] = ["ready", "claimed", "waiting", "done", "cancelled"];

/** What a board shows: the tasks of each column, how many the server counts there, and how to load more. */
export interface BoardColumns {
  groups: Record<Bucket, TaskView[]>;
  totals?: Partial<Record<Bucket, number>>;
  more?: Partial<Record<Bucket, BucketMore>>;
}

/**
 * A saved view's tasks, bucketed (`keepOrder`: the view's own order). The view is one list: `more` loads its next
 * page, whichever column asks for it.
 */
export function useColumns(tasks: TaskView[], keepOrder: boolean, more?: BucketMore): BoardColumns {
  const groups = useMemo(() => groupTasks(tasks, keepOrder), [tasks, keepOrder]);
  return useMemo(() => ({ groups, ...(more ? { more: { ready: more, claimed: more, waiting: more, done: more, cancelled: more } } : {}) }), [groups, more]);
}

export function BoardView({
  loading,
  columns,
  selected,
  onOpen,
  onNew,
  bare,
  showMenu,
}: {
  loading: boolean;
  columns: BoardColumns;
  selected: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  /** Nothing on the board at all. */
  bare: boolean;
  showMenu: boolean;
}) {
  const [params, setParams] = useSearchParams();
  const active = activeColumn(params);
  const root = useRef<HTMLDivElement>(null);
  const strip = useRef<HTMLDivElement>(null);
  // the column a header tap is scrolling to: swiping past the ones in between must not rewrite `?col=` on the way
  const heading = useRef<{ id: Bucket; at: number } | null>(null);
  useFillTop(root);

  const setCol = (id: Bucket) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (id === BOARD_COLS[0]) n.delete("col");
        else n.set("col", id);
        return n;
      },
      { replace: true },
    );

  // a phone opens on the column in `?col=`
  useLayoutEffect(() => {
    if (loading || bare || sideBySide()) return;
    strip.current?.children[BOARD_COLS.indexOf(active)]?.scrollIntoView({ behavior: "instant", block: "nearest", inline: "start" });
    // only when the board appears: later changes of `?col=` come from the strip itself
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading, bare]);

  if (bare && !loading) {
    return (
      <EmptyState title="The board is bare" action={<Button onClick={onNew}>New task</Button>}>
        Tasks go from ready to scoop, to on the fire, to served; a task waiting on a person or a dependency sits in between, and tossed ones are kept aside.
      </EmptyState>
    );
  }

  const go = (id: Bucket) => {
    heading.current = { id, at: Date.now() };
    setCol(id);
    strip.current?.children[BOARD_COLS.indexOf(id)]?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "start" });
  };
  // a swipe: the column now in view becomes the current one
  const onScroll = () => {
    const el = strip.current;
    const first = el?.children[0];
    const second = el?.children[1];
    if (!el || !first || !second || sideBySide()) return;
    const step = second.getBoundingClientRect().left - first.getBoundingClientRect().left;
    const id = BOARD_COLS[Math.max(0, Math.min(BOARD_COLS.length - 1, Math.round(el.scrollLeft / step)))]!;
    const tap = heading.current;
    if (tap && Date.now() - tap.at < HEADING_MS) {
      if (id === tap.id) heading.current = null;
      return;
    }
    heading.current = null;
    if (id !== active) setCol(id);
  };

  const groups = columns.groups;
  return (
    <div
      ref={root}
      className="flex h-[max(24rem,calc(100dvh-var(--fill-top,12rem)-4.5rem-env(safe-area-inset-bottom)))] min-w-0 -mb-[calc(5.5rem+env(safe-area-inset-bottom))] flex-col md:h-[max(24rem,calc(100dvh-var(--fill-top,12rem)-0.75rem))] md:-mb-16"
    >
      {/* phones: every column's name and count, the current one lit; a tap scrolls to it */}
      <div role="tablist" aria-label="Board columns" className="mb-2 shrink-0 sm:hidden">
        <div className="grid grid-cols-5 gap-0.5 rounded-xl bg-muted p-1">
          {BOARD_COLS.map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={active === id}
              onClick={() => go(id)}
              className={cn(
                "flex min-h-10 min-w-0 flex-col items-center justify-center rounded-lg px-0.5 text-[11.5px] leading-tight font-medium transition-colors",
                active === id ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              <span className="flex min-w-0 items-center gap-1">
                <span className={cn("size-1.5 shrink-0 rounded-full", BUCKET_DOT[id], id === "claimed" && groups[id].length > 0 && "live-dot")} aria-hidden="true" />
                {BOARD_SHORT[id]}
              </span>
              <span className="font-mono text-[11px] tabular-nums">{columns.totals?.[id] ?? groups[id].length}</span>
            </button>
          ))}
        </div>
      </div>
      {/* the columns in a row that scrolls sideways (one per screen, snapping, on a phone); each scrolls up and down on its own */}
      <div
        ref={strip}
        role="region"
        aria-label="Board"
        aria-busy={loading || undefined}
        tabIndex={0}
        onScroll={onScroll}
        className="scrollbar-thin relative -mx-4 flex min-h-0 flex-1 snap-x snap-mandatory scroll-px-4 gap-3 overflow-x-auto overflow-y-hidden overscroll-x-contain px-4 pb-1 focus-visible:outline-none sm:snap-proximity md:-mx-5 md:scroll-px-5 md:gap-3.5 md:px-5 lg:-mx-8 lg:scroll-px-8 lg:px-8"
      >
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          const more = columns.more?.[id];
          const total = columns.totals?.[id];
          if (loading) return <ColumnSkeleton key={id} label={meta.label} />;
          return (
            <VirtualColumn
              key={id}
              scroll="own"
              aria-label={meta.label}
              className={cn(COLUMN_WIDTH, id === "cancelled" && "opacity-80")}
              header={
                <h2 className="flex items-center gap-2 px-4 pt-3.5 pb-2.5 text-[15px] max-sm:sr-only">
                  <span className={cn("size-2.5 rounded-full", BUCKET_DOT[id], id === "claimed" && col.length > 0 && "live-dot")} aria-hidden="true" />
                  {meta.label}
                  <span className="font-mono text-[11.5px] font-normal text-muted-foreground">{total ?? col.length}</span>
                </h2>
              }
              bodyClassName="px-2.5"
              items={col}
              itemKey={(t) => t.id}
              empty={
                <>
                  <p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">{more?.has ? "Nothing here yet." : BOARD_EMPTY[id]}</p>
                  <MoreFooter more={more} loaded={0} total={total} />
                </>
              }
              onEndReached={more?.has ? more.load : undefined}
              footer={<MoreFooter more={more} loaded={col.length} total={total} />}
              renderItem={(t) => <TaskCard task={t} selected={selected === t.id} onOpen={onOpen} showMenu={showMenu} />}
            />
          );
        })}
      </div>
    </div>
  );
}

/** How long after a header tap the strip is taken to be still on its way to that column (a smooth scroll). */
const HEADING_MS = 700;

/** A column is one screen wide on a phone, a fixed 19rem from `sm` up; the row scrolls sideways when they do not fit. */
const COLUMN_WIDTH = "shrink-0 basis-full snap-start sm:basis-[19rem]";

/** Five columns side by side (`sm` up) or one per screen (phones): the board strip's own layout, as the CSS has it. */
const sideBySide = () => window.matchMedia("(min-width: 640px)").matches;

/** The column named by `?col=`, the first one when there is none. */
function activeColumn(params: URLSearchParams): Bucket {
  const col = params.get("col");
  return BOARD_COLS.find((id) => id === col) ?? BOARD_COLS[0]!;
}

/**
 * After a column's last card: a spinner while the next page loads, or a button when the end was not reached by
 * scrolling. Under an empty column too: a saved view is one list bucketed here, so a column can be empty while the
 * view has more to load.
 */
function MoreFooter({ more, loaded, total }: { more: BucketMore | undefined; loaded: number; total: number | undefined }) {
  if (!more?.has) return null;
  return (
    <div className="flex justify-center px-2 pb-3">
      <Button type="button" variant="ghost" size="sm" disabled={more.loading} onClick={more.load} className="text-[12.5px] text-muted-foreground">
        {more.loading && <Loader2 className="animate-spin" />}
        {more.loading ? "Loading" : total ? `Show more (${loaded} of ${total})` : "Show more"}
      </Button>
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

const SKELETON_WIDTH = ["w-[88%]", "w-[74%]", "w-[60%]"];

function ColumnSkeleton({ label }: { label: string }) {
  return (
    <section aria-label={`Loading ${label}`} className={cn("space-y-2 rounded-[14px] bg-column p-2.5", COLUMN_WIDTH)}>
      {Array.from({ length: 3 }).map((__, i) => (
        <div key={i} className="space-y-2.5 rounded-[10px] border border-border bg-card p-3">
          <div className="h-2.5 w-12 animate-pulse rounded bg-muted" />
          <div className={cn("h-3.5 animate-pulse rounded bg-muted", SKELETON_WIDTH[i])} />
          <div className="h-2 w-2/3 animate-pulse rounded-full bg-muted" />
        </div>
      ))}
    </section>
  );
}
