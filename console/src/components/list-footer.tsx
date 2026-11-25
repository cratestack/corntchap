import type { ReactNode } from "react";
import { Loader2 } from "lucide-react";
import type { ListSource } from "@/lib/listbox";

/**
 * What closes a paged list that scrolls with the page: "Loading more" while the next page loads, the capped note
 * when the most pages are loaded, "That is everything" at the true end (as `ListPage` shows them), else nothing.
 */
export function listFooter<T>(source: ListSource<T>): ReactNode {
  if (source.isFetchingNextPage) {
    return (
      <p role="status" className="flex items-center justify-center gap-2 py-4 text-[12.5px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
      </p>
    );
  }
  if (source.capped) return <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is as far as this goes. Refine the filters to see more.</p>;
  if (!source.hasNextPage && source.items.length > 0) return <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is everything.</p>;
  return null;
}

/** Fetch the next page of a source if it has one and is not already on it: what a list's end coming into view does. */
export function loadMore<T>(source: ListSource<T>): () => void {
  return () => {
    if (source.hasNextPage && !source.isFetchingNextPage) source.fetchNextPage();
  };
}
