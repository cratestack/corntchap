import { useLayoutEffect, useMemo, useRef } from "react";
import { useSearchParams } from "react-router";
import type { StateSnapshot, VirtuosoHandle } from "react-virtuoso";
import type { TaskView } from "@corntchap/api";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { BucketMore } from "@/lib/board";
import { BUCKETS, type Bucket } from "@/lib/format";
import { useAtLeastWide } from "@/lib/use-element";
import { useStoreValue } from "@/lib/use-store-value";
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
  const groups = columns.groups;
  // five columns side by side only when all five fit; otherwise one column at a time under a tab strip. Measured on
  // the board's own box (not the window): a wrapping grid would put a column below one that pages forever.
  const { fits: allFit, ref } = useAtLeastWide<HTMLDivElement>(ALL_COLUMNS_REM);
  if (bare && !loading) {
    return (
      <EmptyState title="The board is bare" action={<Button onClick={onNew}>New task</Button>}>
        Tasks go from ready to scoop, to on the fire, to served; a task waiting on a person or a dependency sits in between, and tossed ones are kept aside.
      </EmptyState>
    );
  }
  return (
    <div ref={ref} className="min-w-0">
      {loading || allFit === null ? (
        <BoardSkeleton across={allFit ? BOARD_COLS.length : 1} />
      ) : allFit ? (
        <WideBoard columns={columns} selected={selected} onOpen={onOpen} showMenu={showMenu} groups={groups} />
      ) : (
        <TabBoard columns={columns} selected={selected} onOpen={onOpen} showMenu={showMenu} />
      )}
    </div>
  );
}

/** The narrowest a column gets side by side. */
const COLUMN_MIN_REM = 17;
const COLUMN_GAP_REM = 0.875;
/** The room five columns need side by side (5 × 17rem and four 0.875rem gaps: 1416px). */
const ALL_COLUMNS_REM = BOARD_COLS.length * COLUMN_MIN_REM + (BOARD_COLS.length - 1) * COLUMN_GAP_REM;

/** All five columns side by side, each as tall as its cards; the page scrolls through them. */
function WideBoard({
  columns,
  groups,
  selected,
  onOpen,
  showMenu,
}: {
  columns: BoardColumns;
  groups: BoardColumns["groups"];
  selected: string | null;
  onOpen: (id: string) => void;
  showMenu: boolean;
}) {
  return (
    <div role="region" aria-label="Board" className="grid grid-cols-5 items-start gap-3.5">
      {BOARD_COLS.map((id) => {
        const meta = BUCKETS.find((b) => b.id === id)!;
        const col = groups[id];
        const more = columns.more?.[id];
        return (
          <VirtualColumn
            key={id}
            aria-label={meta.label}
            className={cn(id === "cancelled" && "opacity-80")}
            headerClassName="sticky top-0 z-10 rounded-t-[14px] bg-column"
            bodyClassName="px-2.5 pb-2.5"
            header={
              <h2 className="flex items-center gap-2 px-4 pt-3.5 pb-2.5 text-[15px]">
                <span className={cn("size-2.5 rounded-full", BUCKET_DOT[id], id === "claimed" && "live-dot")} aria-hidden="true" />
                {meta.label}
                <span className="font-mono text-[11.5px] font-normal text-muted-foreground">{columns.totals?.[id] ?? col.length}</span>
              </h2>
            }
            items={col}
            itemKey={(t) => t.id}
            empty={
              <>
                <p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">{more?.has ? "Nothing here yet." : "Nothing here."}</p>
                <MoreFooter more={more} loaded={0} total={columns.totals?.[id]} />
              </>
            }
            onEndReached={more?.has ? more.load : undefined}
            footer={<MoreFooter more={more} loaded={col.length} total={columns.totals?.[id]} />}
            renderItem={(t) => <TaskCard task={t} selected={selected === t.id} onOpen={onOpen} showMenu={showMenu} />}
          />
        );
      })}
    </div>
  );
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

function BoardSkeleton({ across }: { across: number }) {
  return (
    <div className={cn("grid items-start gap-3.5", across > 1 ? "grid-cols-5" : "grid-cols-1")} aria-busy="true" aria-label="Loading the board">
      {Array.from({ length: across }).map((_, c) => (
        <div key={c} className="space-y-2 rounded-[14px] bg-column p-2.5">
          {Array.from({ length: 3 }).map((__, i) => (
            <div key={i} className="space-y-2.5 rounded-[10px] border border-border bg-card p-3">
              <div className="h-2.5 w-12 animate-pulse rounded bg-muted" />
              <div className={cn("h-3.5 animate-pulse rounded bg-muted", SKELETON_WIDTH[i])} />
              <div className="h-2 w-2/3 animate-pulse rounded-full bg-muted" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/** The column named by `?col=`, the first one when there is none. */
function activeColumn(params: URLSearchParams): Bucket {
  const col = params.get("col");
  return BOARD_COLS.find((id) => id === col) ?? BOARD_COLS[0]!;
}

/**
 * The board when five columns do not fit side by side (phones, tablets, most laptops): a tab strip, sticky at the top
 * of the page (under the phone's top bar), and one column at a time below it. The page scrolls through the column;
 * each column's scroll position is kept for when its tab is picked again. The tab is in the URL (`?col=`).
 */
function TabBoard({
  columns,
  selected,
  onOpen,
  showMenu,
}: {
  columns: BoardColumns;
  selected: string | null;
  onOpen: (id: string) => void;
  showMenu: boolean;
}) {
  const groups = columns.groups;
  const [params, setParams] = useSearchParams();
  const active = activeColumn(params);
  const tabs = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const list = useRef<VirtuosoHandle>(null);
  // each column as it was left (the page's scroll position and the list's measured heights), and where the next tab change should land
  const { value: left, set: setLeft } = useStoreValue<ReadonlyMap<Bucket, { y: number; snap: StateSnapshot | undefined }>>(new Map());
  const landing = useRef<number | null>(null);

  const pick = (id: Bucket) => {
    if (id === active) return;
    const y = window.scrollY;
    list.current?.getState((snap) => setLeft(new Map(left).set(active, { y, snap })));
    // a column not seen before starts at its top, just under the tab strip (or where the page is, above it)
    const top = panel.current && tabs.current ? y + panel.current.getBoundingClientRect().top - tabs.current.getBoundingClientRect().bottom : y;
    landing.current = left.get(id)?.y ?? Math.min(y, top);
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (id === BOARD_COLS[0]) n.delete("col");
        else n.set("col", id);
        return n;
      },
      { replace: true },
    );
  };

  // the new column's list measures itself over a few frames: keep asking for the position until the page is tall enough
  useLayoutEffect(() => {
    const y = landing.current;
    if (y == null) return;
    let tries = 0;
    let frame = 0;
    const go = () => {
      window.scrollTo(0, y);
      if (Math.abs(window.scrollY - y) <= 1 || ++tries > 20) {
        landing.current = null;
        return;
      }
      frame = requestAnimationFrame(go);
    };
    go();
    return () => cancelAnimationFrame(frame);
  }, [active]);

  const meta = BUCKETS.find((b) => b.id === active)!;
  const col = groups[active];
  const more = columns.more?.[active];
  const count = columns.totals?.[active] ?? col.length;
  return (
    <div>
      <div
        ref={tabs}
        role="tablist"
        aria-label="Board columns"
        className="sticky top-[calc(3.5rem+env(safe-area-inset-top))] z-20 -mx-4 bg-background/95 px-4 pb-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/80 md:top-0 md:-mx-5 md:px-5 md:pt-2 lg:-mx-8 lg:px-8"
      >
        <div className="grid grid-cols-5 gap-0.5 rounded-xl bg-muted p-1">
          {BOARD_COLS.map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              id={`board-tab-${id}`}
              aria-selected={active === id}
              aria-controls="board-panel"
              onClick={() => pick(id)}
              className={cn(
                "flex min-h-10 min-w-0 flex-col items-center justify-center rounded-lg px-0.5 text-[11.5px] leading-tight font-medium transition-colors sm:flex-row sm:gap-2 sm:text-[13px]",
                active === id ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              <span className="flex min-w-0 items-center gap-1 sm:gap-1.5">
                <span className={cn("size-1.5 shrink-0 rounded-full sm:size-2", BUCKET_DOT[id], id === "claimed" && groups[id].length > 0 && "live-dot")} aria-hidden="true" />
                <span className="lg:hidden">{BOARD_SHORT[id]}</span>
                <span className="truncate max-lg:hidden">{BUCKETS.find((b) => b.id === id)!.label}</span>
              </span>
              <span className="font-mono text-[11px] tabular-nums sm:text-[11.5px] sm:text-muted-foreground">{columns.totals?.[id] ?? groups[id].length}</span>
            </button>
          ))}
        </div>
      </div>
      <div ref={panel} id="board-panel" role="tabpanel" aria-labelledby={`board-tab-${active}`}>
        {/* keyed: each column's list starts fresh, so only the column in view asks for more */}
        <VirtualColumn
          key={active}
          listRef={list}
          restoreStateFrom={left.get(active)?.snap}
          aria-label={`${meta.label}, ${count}`}
          className={cn("p-2", active === "cancelled" && "opacity-80")}
          header={
            <h2 className="sr-only">
              {meta.label}, {count}
            </h2>
          }
          items={col}
          itemKey={(t) => t.id}
          empty={
            <>
              <p className="px-2 py-8 text-center text-[13px] text-muted-foreground">{more?.has ? "Nothing here yet." : BOARD_EMPTY[active]}</p>
              <MoreFooter more={more} loaded={0} total={columns.totals?.[active]} />
            </>
          }
          onEndReached={more?.has ? more.load : undefined}
          footer={<MoreFooter more={more} loaded={col.length} total={columns.totals?.[active]} />}
          renderItem={(t) => <TaskCard task={t} selected={selected === t.id} onOpen={onOpen} showMenu={showMenu} />}
        />
      </div>
    </div>
  );
}
