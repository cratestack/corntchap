import { useCallback, type Key, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import type { ListSource } from "@/lib/listbox";

import { ShowMore, loadMore, type MoreMode } from "./list-footer";
import { VirtualList } from "./ui/virtual-list";

/**
 * After a plain (not virtualized) list that a source feeds: reads the next page when it scrolls into view, and says
 * so while it does and when the end is reached. For lists whose rows are drawn by a component that takes an array.
 */
export function LoadMore({ source }: { source: Pick<ListSource<unknown>, "hasNextPage" | "isFetchingNextPage" | "fetchNextPage" | "items"> }) {
  const { hasNextPage, isFetchingNextPage, fetchNextPage, items } = source;
  const count = items.length;
  // a new observer after every page, so a sentinel that is still in view asks for the next one
  const sentinel = useCallback(
    (el: HTMLElement | null) => {
      if (!el || !hasNextPage || isFetchingNextPage) return;
      const seen = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && fetchNextPage(), { rootMargin: "400px" });
      seen.observe(el);
      return () => seen.disconnect();
    },
    // `count` is a dependency on purpose: it re-arms the observer when a page arrived
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [hasNextPage, isFetchingNextPage, fetchNextPage, count],
  );
  if (isFetchingNextPage) {
    return (
      <p role="status" className="flex items-center justify-center gap-2 py-2 text-[12.5px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
      </p>
    );
  }
  if (!hasNextPage) return null;
  return <div ref={sentinel} aria-hidden="true" className="h-4" />;
}

export interface SourceListProps<T> {
  source: ListSource<T>;
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, index: number) => ReactNode;
  /** Shown when there is nothing to list. */
  empty: ReactNode;
  /** The list's accessible name. */
  label: string;
  /** Skeleton rows while the first page loads (default 2). */
  skeletonRows?: number;
  /** On every row's wrapper (default: a hairline between rows). */
  itemClassName?: string;
  /**
   * How it reads its next page: `"scroll"` (default) as its end scrolls into view, for the last list on a page;
   * `"button"` with a "Show more" button, for any list with more content below it (docs/design.md "Lists").
   */
  more?: MoreMode;
}

/**
 * A paged list inside a page section (a details page's Group): virtualized against the window, loading more as the
 * end comes into view, with its loading, empty, error, loading-more, capped and end states. `ListPage` is the same
 * for a whole page.
 */
export function SourceList<T>({ source, itemKey, renderItem, empty, label, skeletonRows = 2, itemClassName = "border-b border-border/70 last:border-b-0", more = "scroll" }: SourceListProps<T>) {
  const { items } = source;
  if (source.isLoading && items.length === 0) {
    return (
      <div role="status" aria-busy="true" aria-label={`Loading ${label}`} className="space-y-2 p-3">
        {Array.from({ length: skeletonRows }, (_, i) => (
          <div key={i} className="h-11 animate-pulse rounded-lg bg-muted" />
        ))}
      </div>
    );
  }
  if (source.error && items.length === 0) {
    return (
      <p role="alert" className="px-4 py-3 text-[13px] text-destructive">
        {source.error instanceof Error ? source.error.message : "Could not load this list."}
      </p>
    );
  }
  if (items.length === 0) return <div className="px-4 py-3 text-[13px] text-muted-foreground">{empty}</div>;
  const footer = source.isFetchingNextPage && more === "scroll" ? (
    <p role="status" className="flex items-center justify-center gap-2 py-3 text-[12.5px] text-muted-foreground">
      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
    </p>
  ) : more === "button" && source.hasNextPage && !source.capped ? (
    <ShowMore source={source} />
  ) : source.capped ? (
    <p className="py-3 text-center text-[12.5px] text-muted-foreground">That is as far as this goes.</p>
  ) : !source.hasNextPage ? (
    <p className="py-3 text-center text-[12.5px] text-muted-foreground">That is everything.</p>
  ) : null;
  return (
    <VirtualList<T>
      items={items}
      itemKey={itemKey}
      renderItem={renderItem}
      scroll="window"
      aria-label={label}
      onEndReached={loadMore(source, more)}
      footer={footer}
      itemClassName={itemClassName}
      defaultItemHeight={56}
    />
  );
}
