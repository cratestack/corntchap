import { useCallback, useLayoutEffect, useMemo, useRef, useSyncExternalStore, type ReactNode, type UIEvent } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { ArrowDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { OVERSCAN_PX, virtualComponents, type VirtualContext } from "./virtual-shared";

/**
 * Virtuoso's index of the first item. Prepending older messages lowers it by their count, which is how the list
 * keeps the message you are reading where it is.
 */
export const START_INDEX = 1_000_000;

export interface VirtualThreadProps<T> {
  /** Oldest first. */
  items: readonly T[];
  /** A stable key per item (a message id): it is how prepended rows are told from replaced ones. */
  itemKey: (item: T) => string;
  renderItem: (item: T, index: number) => ReactNode;
  /** The newest message's sequence number: when it grows while you are scrolled up, "New messages" shows. */
  lastSeq: number;
  /** Older messages exist on the server. */
  hasOlder?: boolean;
  loadingOlder?: boolean;
  /** Fires when the top comes into view and `hasOlder`. */
  onLoadOlder?: (() => void) | undefined;
  /** Shown instead of the thread when there are no items. */
  empty?: ReactNode;
  /** On every row's wrapper: width, centering and spacing as padding (margins are not measured). */
  itemClassName?: string;
  className?: string;
  "aria-label"?: string;
}

/** How close to the bottom still counts as "at the bottom", in px. */
const AT_BOTTOM_PX = 64;
/** After the user's own scroll input, new messages are not followed (and Virtuoso's bottom report is not trusted) for this long. */
const INPUT_QUIET_MS = 350;

/**
 * Where the reader is, kept in a tiny external store, not component state: `atBottom` is ours (Virtuoso's
 * `atBottomStateChange`, flipped to false at once by a user scroll up), `seen` is the last message seen while at
 * the bottom, and `inputAt` is when the user last scrolled, which `follow` must not fight.
 */
function useBottom(lastSeq: number) {
  const store = useMemo(() => {
    let snap = { atBottom: true, seen: lastSeq };
    let inputAt = -Infinity;
    let lastTop = 0;
    const listeners = new Set<() => void>();
    const set = (next: typeof snap) => {
      if (next.atBottom === snap.atBottom && next.seen === snap.seen) return;
      snap = next;
      listeners.forEach((l) => l());
    };
    const quiet = () => performance.now() - inputAt > INPUT_QUIET_MS;
    return {
      get: () => snap,
      subscribe: (l: () => void) => {
        listeners.add(l);
        return () => void listeners.delete(l);
      },
      /** Virtuoso reports whether the bottom is in view. Right after the user scrolled up, a "yes" is stale. */
      bottom: (atBottom: boolean, last: number) => {
        if (atBottom && !snap.atBottom && !quiet()) return;
        set({ atBottom, seen: atBottom ? last : snap.seen });
      },
      /** A newer message arrived: while at the bottom it is followed (seen), otherwise it is news. */
      arrived: (last: number) => set({ atBottom: snap.atBottom, seen: snap.atBottom ? last : snap.seen }),
      /** Wheel, touch or key input: the user is driving the scroll. */
      input: () => {
        inputAt = performance.now();
      },
      /** A scroll event of the list's scroller: moving up and away from the bottom is the user leaving it. */
      scrolled: (el: HTMLElement) => {
        const top = el.scrollTop;
        const up = top < lastTop - 1;
        lastTop = top;
        if (up && el.scrollHeight - top - el.clientHeight > AT_BOTTOM_PX) {
          inputAt = performance.now();
          set({ atBottom: false, seen: snap.seen });
        }
      },
      /** Virtuoso's `followOutput`: only when we are at the bottom and the user is not scrolling. */
      follow: () => (snap.atBottom && quiet() ? ("auto" as const) : false),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const snap = useSyncExternalStore(store.subscribe, store.get, store.get);
  return { atBottom: snap.atBottom, seen: snap.seen, store };
}

/**
 * A chat thread: oldest message at the top, newest at the bottom, starting scrolled to the bottom. New messages
 * are followed while you are at the bottom; scrolled up, a "New messages" pill says there are some. Reaching the
 * top asks for older ones, and prepending them keeps what you were reading in place.
 */
export function VirtualThread<T>({ items, itemKey, renderItem, lastSeq, hasOlder, loadingOlder, onLoadOlder, empty, itemClassName, className, "aria-label": label }: VirtualThreadProps<T>) {
  const ref = useRef<VirtuosoHandle>(null);
  const bottom = useBottom(lastSeq);
  const lastSeqRef = useRef(lastSeq);
  lastSeqRef.current = lastSeq;

  // firstItemIndex: what was the first row at the last commit tells how many rows were prepended since
  const firstKey = items.length > 0 ? itemKey(items[0] as T) : null;
  const committed = useRef<{ key: string | null; index: number }>({ key: null, index: START_INDEX });
  const firstIndex = useMemo(() => {
    const before = committed.current;
    if (firstKey === null || before.key === null) return START_INDEX;
    if (before.key === firstKey) return before.index;
    const at = items.findIndex((it) => itemKey(it) === before.key);
    // the old first row is further down: that many rows came in above it; gone: another list altogether
    return at > 0 ? before.index - at : START_INDEX;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [firstKey, items]);
  useLayoutEffect(() => {
    committed.current = { key: firstKey, index: firstIndex };
  }, [firstKey, firstIndex]);

  const { store } = bottom;
  useLayoutEffect(() => store.arrived(lastSeq), [lastSeq, store]);

  const context = useMemo<VirtualContext>(
    () => ({
      itemClassName,
      label,
      header: loadingOlder ? (
        <p role="status" className="grid h-10 place-items-center text-[12px] text-muted-foreground">
          Loading earlier messages
        </p>
      ) : hasOlder ? (
        <div className="h-10" />
      ) : null,
      footer: <div className="h-3" />,
    }),
    [itemClassName, label, loadingOlder, hasOlder],
  );
  const startReached = useCallback(() => {
    if (hasOlder && !loadingOlder) onLoadOlder?.();
  }, [hasOlder, loadingOlder, onLoadOlder]);
  const atBottomChange = useCallback((at: boolean) => store.bottom(at, lastSeqRef.current), [store]);
  const onScroll = useCallback(
    (e: UIEvent<HTMLDivElement>) => {
      if (e.target instanceof HTMLElement && e.target.hasAttribute("data-virtuoso-scroller")) store.scrolled(e.target);
    },
    [store],
  );

  if (items.length === 0) return <>{empty}</>;
  const news = !bottom.atBottom && lastSeq > bottom.seen;
  return (
    <div
      className={cn("relative h-full min-h-0", className)}
      onScrollCapture={onScroll}
      onWheelCapture={store.input}
      onTouchStartCapture={store.input}
      onTouchMoveCapture={store.input}
      onKeyDownCapture={store.input}
    >
      <Virtuoso<T, VirtualContext>
        ref={ref}
        data={items}
        context={context}
        components={virtualComponents}
        firstItemIndex={firstIndex}
        initialTopMostItemIndex={{ index: items.length - 1, align: "end" }}
        alignToBottom
        computeItemKey={(_index, item) => itemKey(item)}
        itemContent={(index, item) => renderItem(item, index - firstIndex)}
        increaseViewportBy={{ top: OVERSCAN_PX, bottom: OVERSCAN_PX }}
        defaultItemHeight={72}
        atBottomThreshold={AT_BOTTOM_PX}
        atBottomStateChange={atBottomChange}
        followOutput={store.follow}
        startReached={startReached}
        className="scrollbar-thin overscroll-contain"
      />
      {news && (
        <button
          type="button"
          onClick={() => ref.current?.scrollToIndex({ index: "LAST", align: "end", behavior: "smooth" })}
          className="absolute right-4 bottom-3 inline-flex h-9 items-center gap-1.5 rounded-full border border-border bg-card px-3 text-[12.5px] font-medium shadow-md"
        >
          <ArrowDown className="size-3.5" aria-hidden="true" /> New messages
        </button>
      )}
    </div>
  );
}
