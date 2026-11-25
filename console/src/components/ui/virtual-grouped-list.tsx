import { useMemo, type Key, type ReactNode, type Ref } from "react";
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
  const context = useMemo<VirtualContext>(() => ({ itemClassName, footer, stickyClassName }), [itemClassName, footer, stickyClassName]);
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
      computeItemKey={(position) => listKeys[position] ?? position}
      groupContent={(gi) => renderGroup(groups[gi]!, gi)}
      itemContent={(index, gi) => renderItem(items[index]!, index, groups[gi]!)}
      increaseViewportBy={{ top: OVERSCAN_PX, bottom: OVERSCAN_PX }}
      {...(defaultItemHeight ? { defaultItemHeight } : {})}
      {...(onEndReached ? { endReached: onEndReached } : {})}
      {...scrollProps(scroll)}
      {...(label ? { "aria-label": label } : {})}
      {...(scroll === "own" ? { className: "scrollbar-thin overscroll-contain" } : className ? { className } : {})}
    />
  );
  return scroll === "own" ? <div className={cn("h-full min-h-0", className)}>{list}</div> : list;
}
