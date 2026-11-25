import { useId, type Key, type ReactNode, type Ref } from "react";
import type { StateSnapshot, VirtuosoHandle } from "react-virtuoso";
import { cn } from "@/lib/utils";
import { VirtualList } from "./virtual-list";
import type { VirtualScroll } from "./virtual-shared";

export interface VirtualColumnProps<T> {
  /** The column's title row (name, count). */
  header: ReactNode;
  /** On the header's wrapper, e.g. `sticky top-0 z-10 bg-column` to keep the title in view while the page scrolls. */
  headerClassName?: string | undefined;
  items: readonly T[];
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, index: number) => ReactNode;
  empty?: ReactNode;
  /** Fires when the end of the cards comes into view, to load the next page. */
  onEndReached?: (() => void) | undefined;
  /** After the last card: a spinner or a "load more" button. */
  footer?: ReactNode;
  /** The list's handle (`getState` to remember where it was scrolled) and the snapshot to start from. */
  listRef?: Ref<VirtuosoHandle>;
  restoreStateFrom?: StateSnapshot | undefined;
  /**
   * `"window"` (default): the column is as tall as its cards and the page scrolls through it.
   * `"own"`: the column fills its parent's height and its cards scroll inside it (the board).
   */
  scroll?: Extract<VirtualScroll, string>;
  /** On the wrapper around the cards (padding). */
  bodyClassName?: string | undefined;
  /** The column surface; size it with the grid or flex it sits in. */
  className?: string | undefined;
  "aria-label"?: string;
}

/**
 * One board column: a header slot and a virtualized stack of cards. By default the column is as tall as its cards and
 * the page scrolls through it; with `scroll="own"` it fills its parent and the cards scroll inside it, under the header.
 */
export function VirtualColumn<T>({ header, headerClassName, items, itemKey, renderItem, empty, onEndReached, footer, listRef, restoreStateFrom, scroll = "window", bodyClassName, className, "aria-label": label }: VirtualColumnProps<T>) {
  const id = useId();
  return (
    <section aria-labelledby={label ? undefined : id} aria-label={label} className={cn("flex min-w-0 flex-col rounded-[14px] bg-column", scroll === "own" && "h-full min-h-0", className)}>
      <div id={id} className={headerClassName}>
        {header}
      </div>
      <div className={cn(scroll === "own" && "min-h-0 flex-1", bodyClassName)}>
        <VirtualList {...(listRef ? { ref: listRef } : {})} {...(restoreStateFrom ? { restoreStateFrom } : {})} items={items} itemKey={itemKey} renderItem={renderItem} scroll={scroll} empty={empty} onEndReached={onEndReached} footer={footer} itemClassName="pb-2" defaultItemHeight={96} />
      </div>
    </section>
  );
}
