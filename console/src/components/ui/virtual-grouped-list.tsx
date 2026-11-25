import { useMemo, useState, type Key, type ReactNode, type Ref } from "react";
import { GroupedVirtuoso, type GroupedVirtuosoHandle } from "react-virtuoso";
import { cn } from "@/lib/utils";
import { OVERSCAN_PX, scrollProps, virtualComponents, type VirtualContext, type VirtualScroll } from "./virtual-shared";

export interface VirtualGroup {
  key: string;
  label: string;
  /** How many of the flat `items` belong to this group. The counts must add up to `items.length`. */
  count: number;
}

export interface VirtualGroupedListProps<T> {
  groups: readonly VirtualGroup[];
  /** Every item, flat, in group order. */
  items: readonly T[];
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, index: number, group: VirtualGroup) => ReactNode;
  /** The sticky header; give it an opaque background so rows slide under it. */
  renderGroup: (group: VirtualGroup, index: number) => ReactNode;
  scroll?: VirtualScroll;
  onEndReached?: (() => void) | undefined;
  footer?: ReactNode;
  empty?: ReactNode;
  className?: string | undefined;
  itemClassName?: string | undefined;
  /** Applied to the sticky header layer, e.g. `top-14!` to stick below a fixed bar (the `!` beats Virtuoso's own `top: 0`). */
  stickyClassName?: string | undefined;
  defaultItemHeight?: number | undefined;
  "aria-label"?: string;
  ref?: Ref<GroupedVirtuosoHandle>;
}

/** A virtualized list with sticky group headers (days, buckets). */
export function VirtualGroupedList<T>({
  groups,
  items,
  itemKey,
  renderItem,
  renderGroup,
  scroll = "own",
  onEndReached,
  footer,
  empty,
  className,
  itemClassName,
  stickyClassName,
  defaultItemHeight,
  "aria-label": label,
  ref,
}: VirtualGroupedListProps<T>) {
  const context = useMemo<VirtualContext>(() => ({ itemClassName, footer, stickyClassName, label }), [itemClassName, footer, stickyClassName, label]);
  const first = usePrependAnchor(items, itemKey);
  const groupCounts = useMemo(() => groups.map((g) => g.count), [groups]);
  // Virtuoso keys by position in the list, headers included: a group's header, then its items
  const listKeys = useMemo(() => {
    const keys: Key[] = [];
    let at = 0;
    for (const g of groups) {
      keys.push(`group:${g.key}`);
      for (let i = 0; i < g.count; i++, at++) keys.push(itemKey(items[at]!, at));
    }
    return keys;
  }, [groups, items, itemKey]);
  if (items.length === 0 && empty) return <>{empty}</>;
  if (typeof scroll === "object" && !scroll.parent) return null;
  const list = (
    <GroupedVirtuoso<T, VirtualContext>
      {...(ref ? { ref } : {})}
      groupCounts={groupCounts}
      context={context}
      components={virtualComponents}
      firstItemIndex={first.index}
      atTopStateChange={first.setAtTop}
      computeItemKey={(position) => listKeys[position - first.index] ?? position}
      groupContent={(gi) => (groups[gi] ? renderGroup(groups[gi], gi) : null)}
      itemContent={(index, gi) => {
        // Virtuoso can render once more with its previous counts after the items change (shrunk by a filter, or
        // prepended to and cut at the end): skip the rows that no longer exist rather than crash on them
        const at = index - first.index;
        const item = items[at];
        const group = groups[gi];
        return item === undefined || group === undefined ? null : renderItem(item, at, group);
      }}
      increaseViewportBy={{ top: OVERSCAN_PX, bottom: OVERSCAN_PX }}
      {...(defaultItemHeight ? { defaultItemHeight } : {})}
      {...(onEndReached ? { endReached: onEndReached } : {})}
      {...scrollProps(scroll)}
      {...(scroll === "own" ? { className: "scrollbar-thin overscroll-contain" } : className ? { className } : {})}
    />
  );
  return scroll === "own" ? <div className={cn("h-full min-h-0", className)}>{list}</div> : list;
}

/** Far from zero, so `firstItemIndex` can count down for every row ever prepended. */
const FIRST_INDEX = 1_000_000_000;

/**
 * Keeps the rows under the reader still when new ones arrive on top (a live feed) while the list is scrolled down:
 * Virtuoso shifts its scroll position when `firstItemIndex` drops by the number of rows prepended. The browser's own
 * scroll anchoring cannot do it, Virtuoso turns it off on its rows. At the top of the list nothing shifts, so the new
 * rows show up there. A prepend is recognised by the previous first two rows reappearing, in order, further down
 * (a filter change that merely moves the old first row does not count).
 */
function usePrependAnchor<T>(items: readonly T[], itemKey: (item: T, index: number) => Key) {
  const keyAt = (i: number) => (i < items.length ? itemKey(items[i]!, i) : null);
  const head = [keyAt(0), keyAt(1)] as const;
  const [atTop, setAtTop] = useState(true);
  const [anchor, setAnchor] = useState({ head, index: FIRST_INDEX });
  let index = anchor.index;
  if (anchor.head[0] !== head[0] || anchor.head[1] !== head[1]) {
    let prepended = 0;
    if (!atTop && anchor.head[0] !== null) {
      for (let i = 1; i < items.length; i++) {
        if (keyAt(i) === anchor.head[0]) {
          if (anchor.head[1] === null || keyAt(i + 1) === anchor.head[1]) prepended = i;
          break;
        }
      }
    }
    index = Math.max(0, anchor.index - prepended);
    setAnchor({ head, index });
  }
  return { index, setAtTop };
}
