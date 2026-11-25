import { useCallback, useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ChangeView } from "@corntchap/api";
import { client } from "./api";

/** Events asked for at a time when reading back. */
const OLDER_PAGE = 200;

interface Older {
  /** Oldest first. */
  events: ChangeView[];
  /** Events older than the oldest one known may exist; null until a read back says. */
  older: boolean | null;
}

/**
 * A pot's activity history behind what the live feed keeps: `changesSince` with `before`, a page at a time,
 * kept in the query cache (outside `["data"]`, so live events do not refetch it). `live` is the live feed's
 * events of the pot, oldest first; `mayHaveOlder` says the feed was cut (it keeps only the latest 400), and `floor`
 * is the oldest seq the live feed holds (of any pot): a pot with nothing in the feed reads back from there.
 */
export function useOlderEvents(pot: string, live: ChangeView[], mayHaveOlder: boolean, floor: number | null = null) {
  const qc = useQueryClient();
  const key = useMemo(() => ["older-events", pot] as const, [pot]);
  const state = useQuery({ queryKey: key, queryFn: (): Older => ({ events: [], older: null }), staleTime: Infinity, gcTime: 600_000 });
  const had = state.data;
  const events = useMemo(() => {
    const seen = new Set(live.map((e) => e.seq));
    return [...(had?.events ?? []).filter((e) => !seen.has(e.seq)), ...live];
  }, [had, live]);
  const first = events[0]?.seq ?? floor;
  const load = useMutation({
    mutationFn: async () => {
      if (first === null) return;
      const feed = await client.procedures.changesSince({ cursor: 0, project: pot, limit: OLDER_PAGE, before: first });
      qc.setQueryData<Older>(key, (cur) => ({ events: [...feed.events, ...(cur?.events ?? [])], older: feed.older }));
    },
  });
  const run = load.mutate;
  return {
    events,
    hasOlder: first !== null && (had?.older ?? mayHaveOlder),
    loadingOlder: load.isPending,
    loadOlder: useCallback(() => run(), [run]),
  };
}
