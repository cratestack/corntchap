import { useCallback, useId, useMemo, type Key, type ReactNode } from "react";
import { Virtuoso } from "react-virtuoso";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useListbox, type ListboxController, type ListSource } from "@/lib/listbox";
import { listboxComponents, type ListboxBinding, type VirtualContext } from "./virtual-shared";

/** Height of one option row (`min-h-10`), so a short list can be as tall as its rows. */
const ROW_REM = 2.5;
const HEIGHTS: Record<number, string> = { 1: "h-10", 2: "h-20", 3: "h-30", 4: "h-40", 5: "h-50", 6: "h-60", 7: "h-70", 8: "h-80", 9: "h-90", 10: "h-100" };

/** The Tailwind height of a listbox showing `rows` rows (at most `max`): Virtuoso needs a height, and style props are not used. */
export function listboxHeight(rows: number, max: number): string {
  return HEIGHTS[Math.max(1, Math.min(rows, max, 10))] ?? "h-80";
}

export interface VirtualListboxProps<T> {
  source: ListSource<T>;
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, state: { index: number; selected: boolean; active: boolean }) => ReactNode;
  /** The listbox's accessible name. */
  label: string;
  /** Which options show as selected (`aria-selected`). */
  selected?: ((item: T) => boolean) | undefined;
  multiple?: boolean | undefined;
  onPick?: ((item: T, index: number) => void) | undefined;
  isDisabled?: ((item: T) => boolean) | undefined;
  /** Shown instead of the list when there is nothing (after loading). */
  empty?: ReactNode;
  /** On the wrapper; `h-full` to fill a parent instead of sizing to the rows. */
  className?: string | undefined;
  /** Most rows tall before it scrolls (default 8). */
  rows?: number | undefined;
  /** The `role="listbox"` element's id (a combobox's `aria-controls`); made up when left out. */
  id?: string | undefined;
  /** When a search input owns focus it passes its `useListbox` here; the listbox is then not focusable itself. */
  controller?: ListboxController | undefined;
}

/**
 * A virtualized listbox: `role="listbox"` over `role="option"` rows (`aria-selected`, one active row), loading the
 * next page as the end comes into view. On its own it is focusable and takes the keys (ArrowUp/Down, Home/End,
 * PageUp/PageDown, Enter); inside a combobox the input drives it through `controller`.
 */
export function VirtualListbox<T>({ source, itemKey, renderItem, label, selected, multiple = false, onPick, isDisabled, empty, className, rows = 8, id, controller }: VirtualListboxProps<T>) {
  const made = useId();
  const listId = id ?? `${made}-list`;
  const { items, hasNextPage, isFetchingNextPage, fetchNextPage } = source;
  const loadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);
  const own = useListbox<T>({ items, onPick: (item, i) => onPick?.(item, i), isDisabled, idPrefix: made, fetchMore: loadMore });
  const lb = controller ?? own;

  const binding = useMemo<ListboxBinding>(
    () => ({
      id: listId,
      multiple,
      activeIndex: lb.active,
      optionId: lb.optionId,
      isSelected: (item) => !!selected?.(item as T),
      isDisabled: (item) => !!isDisabled?.(item as T),
      pick: lb.pick,
      hover: (i) => lb.setActive(i, { scroll: false }),
      focusable: controller ? undefined : { activeId: lb.activeId, onKeyDown: lb.onKeyDown },
    }),
    [listId, multiple, lb, selected, isDisabled, controller],
  );

  const { capped } = source;
  const long = items.length > rows;
  const footer = useMemo(
    () =>
      isFetchingNextPage ? (
        <p role="status" className="flex items-center justify-center gap-2 py-2 text-[12.5px] text-muted-foreground">
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
        </p>
      ) : capped ? (
        <p className="py-2 text-center text-[12.5px] text-muted-foreground">That is as far as this goes. Refine the search to see more.</p>
      ) : !hasNextPage && long ? (
        <p className="py-2 text-center text-[12.5px] text-muted-foreground">That is everything.</p>
      ) : null,
    [isFetchingNextPage, capped, hasNextPage, long],
  );

  const context = useMemo<VirtualContext>(
    () => ({ label, footer, listbox: binding, itemClassName: "flex min-h-10 items-center px-2.5 py-1.5 text-sm" }),
    [label, footer, binding],
  );

  if (source.isLoading && items.length === 0) {
    return (
      <div role="status" aria-busy="true" aria-label={`${label}, loading`} className={cn("space-y-1.5 p-2", className)}>
        {Array.from({ length: Math.min(rows, 5) }, (_, i) => (
          <div key={i} className="h-8 animate-pulse rounded-md bg-muted" />
        ))}
      </div>
    );
  }
  if (source.error && items.length === 0) {
    return (
      <p role="alert" className={cn("p-4 text-center text-sm text-destructive", className)}>
        {source.error instanceof Error ? source.error.message : "Could not load the list."}
      </p>
    );
  }
  if (items.length === 0) return <div className={cn("p-4 text-center text-sm text-muted-foreground", className)}>{empty ?? "Nothing matches."}</div>;

  const tall = listboxHeight(items.length + (footer ? 1 : 0), rows);
  return (
    <div className={cn("min-h-0 w-full", tall, className)} aria-busy={source.isLoading || undefined}>
      <Virtuoso<T, VirtualContext>
        ref={lb.listRef as never}
        data={items as T[]}
        context={context}
        components={listboxComponents}
        computeItemKey={(index, item) => itemKey(item, index)}
        itemContent={(index, item) => renderItem(item, { index, selected: !!selected?.(item), active: index === lb.active })}
        defaultItemHeight={ROW_REM * 16}
        increaseViewportBy={{ top: 200, bottom: 400 }}
        endReached={loadMore}
        className="scrollbar-thin overscroll-contain"
      />
    </div>
  );
}
