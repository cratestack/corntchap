import { useId, type Key, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { VirtualList } from "./virtual-list";
import type { VirtualScroll } from "./virtual-shared";

export interface VirtualColumnProps<T> {
  /** The column's title row (name, count); stays put while the cards scroll. */
  header: ReactNode;
  items: readonly T[];
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, index: number) => ReactNode;
  empty?: ReactNode;
  /** Fires when the end of the cards comes into view, to load the next page. */
  onEndReached?: (() => void) | undefined;
  /** After the last card: a spinner or a "load more" button. */
  footer?: ReactNode;
  /**
   * `"own"` (default): a fixed-height scroller; the column fills the height of its parent (desktop board).
   * `"window"`: the page scrolls through the column, which is as tall as its cards (phone snap board).
   */
  scroll?: Extract<VirtualScroll, string>;
  /** The column surface; size it (`w-76 shrink-0`, `w-[85vw] snap-start`). */
  className?: string | undefined;
  "aria-label"?: string;
}

/** One board column: a header slot and a virtualized stack of cards. */
export function VirtualColumn<T>({ header, items, itemKey, renderItem, empty, onEndReached, footer, scroll = "own", className, "aria-label": label }: VirtualColumnProps<T>) {
  const id = useId();
  return (
    <section
      aria-labelledby={label ? undefined : id}
      aria-label={label}
      className={cn("flex flex-col rounded-[14px] bg-column", scroll === "own" && "h-full min-h-0", className)}
    >
      <div id={id}>{header}</div>
      <div className={cn(scroll === "own" && "min-h-0 flex-1")}>
        <VirtualList
          items={items}
          itemKey={itemKey}
          renderItem={renderItem}
          scroll={scroll}
          empty={empty}
          onEndReached={onEndReached}
          footer={footer}
          itemClassName="pb-2"
          defaultItemHeight={96}
          {...(scroll === "own" ? { className: "px-2.5 pb-2.5" } : {})}
        />
      </div>
    </section>
  );
}
