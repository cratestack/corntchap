import { useMemo } from "react";
import type { ChangeView } from "@corntchap/api";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useProcInfinite } from "@/lib/paging";
import { Group, RowSkeleton } from "./bits";
import { EventFeed } from "./event-feed";

/** What the restaurant (the server itself) did about one cook: the last 20 decisions and wakes, older ones on request. */
export function RestaurantLogView({
  events,
  loading = false,
  hasMore = false,
  loadingMore = false,
  onMore,
}: {
  /** Newest first, as the server sends them. */
  events: ChangeView[];
  loading?: boolean;
  hasMore?: boolean;
  loadingMore?: boolean;
  onMore?: (() => void) | undefined;
}) {
  const oldestFirst = useMemo(() => [...events].reverse(), [events]);
  return (
    <Group title="Restaurant log" hint="What the server itself decided about this cook: wakes held back or stopped, tasks it skips, runs it ended.">
      <div className="px-3.5 py-1">
        {loading ? (
          <div className="py-3">
            <RowSkeleton rows={2} />
          </div>
        ) : (
          <EventFeed events={oldestFirst} compact empty="Nothing decided about this cook yet." />
        )}
        {hasMore && (
          <div className="flex justify-center pb-3">
            <Button type="button" variant="outline" size="sm" disabled={loadingMore} onClick={onMore}>
              {loadingMore && <Loader2 className="animate-spin" />}
              Show older
            </Button>
          </div>
        )}
      </div>
    </Group>
  );
}

export function RestaurantLog({ cook }: { cook: string }) {
  const log = useProcInfinite("cookDecisions", { agent: cook }, { getCursor: (p) => p.nextCursor, pageSize: 20 });
  return (
    <RestaurantLogView
      events={log.items}
      loading={log.isLoading}
      hasMore={!!log.hasNextPage}
      loadingMore={log.isFetchingNextPage}
      onMore={() => void log.fetchNextPage()}
    />
  );
}
