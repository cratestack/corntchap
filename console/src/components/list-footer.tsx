import type { ReactNode } from "react";
import { Loader2 } from "lucide-react";
import type { ListSource } from "@/lib/listbox";
import { Button } from "./ui/button";

/**
 * How a list that pages asks for its next page. The page scrolls as one (docs/design.md "Lists"), so only the LAST
 * list on a page may load more as its end scrolls into view (`"scroll"`, the default): a list with more content
 * below it would pull in pages forever and the sections under it could never be reached. Every other list gets an
 * explicit "Show more" button (`"button"`).
 */
export type MoreMode = "scroll" | "button";

/** The button that reads a list's next page (a list that is not the last on its page). */
export function ShowMore<T>({ source, label = "Show more" }: { source: Pick<ListSource<T>, "isFetchingNextPage" | "fetchNextPage"> & Partial<Pick<ListSource<T>, "pending">>; label?: string }) {
  return (
    <div className="flex justify-center py-2">
      <Button type="button" variant="ghost" size="sm" disabled={source.isFetchingNextPage} onClick={() => source.fetchNextPage()} className="text-[12.5px] text-muted-foreground">
        {source.isFetchingNextPage && <Loader2 className="animate-spin" />}
        {source.isFetchingNextPage ? "Loading" : label}
      </Button>
    </div>
  );
}

/**
 * What closes a paged list that scrolls with the page: "Loading more" while the next page loads, a "Show more"
 * button when it has more and is not the page's last list (`more: "button"`), the capped note when the most pages are
 * loaded, "That is everything" at the true end (as `ListPage` shows them), else nothing.
 */
export function listFooter<T>(source: ListSource<T>, more: MoreMode = "scroll"): ReactNode {
  if (source.isFetchingNextPage && more === "scroll") {
    return (
      <p role="status" className="flex items-center justify-center gap-2 py-4 text-[12.5px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
      </p>
    );
  }
  if (more === "button" && source.hasNextPage && !source.capped) return <ShowMore source={source} />;
  if (source.capped) return <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is as far as this goes. Refine the filters to see more.</p>;
  if (!source.hasNextPage && source.items.length > 0) return <p className="py-4 text-center text-[12.5px] text-muted-foreground">That is everything.</p>;
  return null;
}

/**
 * Fetch the next page of a source if it has one and is not already on it: what a list's end coming into view does.
 * Nothing for a list that loads on a button (`more: "button"`).
 */
export function loadMore<T>(source: ListSource<T>, more: MoreMode = "scroll"): (() => void) | undefined {
  if (more === "button") return undefined;
  return () => {
    if (source.hasNextPage && !source.isFetchingNextPage) source.fetchNextPage();
  };
}
