import { useMemo, type Key, type ReactNode, type Ref } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { cn } from "@/lib/utils";
import { OVERSCAN_PX, scrollProps, virtualComponents, type VirtualContext, type VirtualScroll } from "./virtual-shared";

export interface VirtualListProps<T> {
  items: readonly T[];
  /** A stable key per item; keeps measured heights and focus attached to the right row. */
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, index: number) => ReactNode;
  /** Where it scrolls; default `"own"` (fills its parent, so the parent needs a height). */
  scroll?: VirtualScroll;
  /** Fires when the end comes into view, to load the next page. */
  onEndReached?: (() => void) | undefined;
  /** After the last row: a spinner, "load more", or a closing note. */
  footer?: ReactNode;
  /** Shown instead of the list when there are no items. */
  empty?: ReactNode;
  /** On the scroller (`own`: its wrapper, which should carry the height) or the list root (other modes). */
  className?: string | undefined;
  /** On every row's wrapper: padding for spacing (margins are not measured). */
  itemClassName?: string | undefined;
  /** An estimate of a row's height before it is measured; closer is steadier scrollbars. */
  defaultItemHeight?: number | undefined;
  "aria-label"?: string;
  ref?: Ref<VirtuosoHandle>;
}

/** A virtualized, Tailwind-styled list: only the rows near the viewport are in the DOM. */
export function VirtualList<T>({
  items,
  itemKey,
  renderItem,
  scroll = "own",
  onEndReached,
  footer,
  empty,
  className,
  itemClassName,
  defaultItemHeight,
  "aria-label": label,
  ref,
}: VirtualListProps<T>) {
  const context = useMemo<VirtualContext>(() => ({ itemClassName, footer }), [itemClassName, footer]);
  if (items.length === 0 && empty) return <>{empty}</>;
  if (typeof scroll === "object" && !scroll.parent) return null;
  const list = (
    <Virtuoso<T, VirtualContext>
      {...(ref ? { ref } : {})}
      data={items}
      context={context}
      components={virtualComponents}
      computeItemKey={(index, item) => itemKey(item, index)}
      itemContent={(index, item) => renderItem(item, index)}
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
