import { type Key, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ListSource } from "@/lib/listbox";
import { VirtualList } from "./ui/virtual-list";
import { VirtualTable, type VirtualTableColumn } from "./ui/virtual-table";

export interface ListPageProps<T> {
  title: string;
  /** Shown beside the title; default the source's total when it has one. */
  count?: number | string | null | undefined;
  /** Buttons at the end of the header. */
  actions?: ReactNode;
  /** A `FilterBar`, under the header. */
  filters?: ReactNode;
  source: ListSource<T>;
  itemKey: (item: T, index: number) => Key;
  /** A row as a list item; with `columns`, the card a phone shows instead of a table row. */
  renderItem: (item: T, index: number) => ReactNode;
  /** A table from 768px up (a sticky header over the rows); phones still get `renderItem` cards. */
  columns?: readonly VirtualTableColumn<T>[] | undefined;
  /** `"window"` (default): the page scrolls. `"own"`: a scroller that fills its parent. */
  scroll?: "window" | "own" | undefined;
  /** Shown when there is nothing to list. */
  empty: ReactNode;
  /** Shown instead of the default message when the first page failed. */
  error?: ReactNode;
  className?: string | undefined;
}

/**
 * A list page: header (title, count, actions), filters, and a virtualized list or table that pages as the page is
 * scrolled, with its loading, empty, error, loading-more and end states.
 */
export function ListPage<T>({ title, count, actions, filters, source, itemKey, renderItem, columns, scroll = "window", empty, error, className }: ListPageProps<T>) {
  const { items } = source;
  const shownCount = count === undefined ? source.total : count;
  const loadMore = () => {
    if (source.hasNextPage && !source.isFetchingNextPage) source.fetchNextPage();
  };
  const footer = source.isFetchingNextPage ? (
    <p role="status" className="flex items-center justify-center gap-2 py-4 text-[12.5px] text-muted-foreground">
      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
    </p>
  ) : source.capped ? (
    <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is as far as this goes. Refine the filters to see more.</p>
  ) : !source.hasNextPage && items.length > 0 ? (
    <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is everything.</p>
  ) : null;

  let body: ReactNode;
  if (source.isLoading && items.length === 0) {
    body = (
      <div role="status" aria-busy="true" aria-label={`Loading ${title}`} className="space-y-2">
        {Array.from({ length: 6 }, (_, i) => (
          <div key={i} className="h-14 animate-pulse rounded-[14px] bg-muted" />
        ))}
      </div>
    );
  } else if (source.error && items.length === 0) {
    body = (
      <div role="alert" className="rounded-[14px] border border-destructive/40 bg-destructive/5 p-6 text-center text-sm text-destructive">
        {error ?? (source.error instanceof Error ? source.error.message : "Could not load this list.")}
      </div>
    );
  } else if (items.length === 0) {
    body = <div className="rounded-[14px] border border-dashed border-border p-8 text-center text-sm text-muted-foreground">{empty}</div>;
  } else if (columns) {
    body = <VirtualTable<T> rows={items} rowKey={itemKey} columns={columns} card={renderItem} scroll={scroll} onEndReached={loadMore} footer={footer} aria-label={title} />;
  } else {
    body = (
      <div className={cn("rounded-[14px] border border-border bg-card", scroll === "own" && "h-full min-h-0")}>
        <VirtualList<T> items={items} itemKey={itemKey} renderItem={renderItem} scroll={scroll} onEndReached={loadMore} footer={footer} aria-label={title} itemClassName="border-b border-border/70 last:border-b-0" defaultItemHeight={64} />
      </div>
    );
  }

  return (
    <div className={cn("px-4 pt-4 pb-10 md:px-5 md:pt-6 md:pb-16 lg:px-8", scroll === "own" && "flex h-full min-h-0 flex-col", className)}>
      <header className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <h1 className="min-w-0 flex-1 basis-40 text-[26px] leading-[1.1] sm:text-[34px]">
          {title}
          {shownCount != null && <span className="ml-3 align-middle font-mono text-[13px] font-normal text-muted-foreground">{typeof shownCount === "number" ? shownCount.toLocaleString() : shownCount}</span>}
        </h1>
        {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </header>
      {filters && <div className="mt-4">{filters}</div>}
      <div className={cn("mt-5", scroll === "own" && "min-h-0 flex-1")}>{body}</div>
    </div>
  );
}
