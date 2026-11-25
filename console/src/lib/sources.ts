import type { ProcArgs } from "./data";
import { useListQuery, type ListQueryOptions } from "./list-query";
import type { PagedProc } from "./paging";

/** A procedure's filters: its arguments without the paging (`page`) and the search text (`q`, typed live through the hook). */
type F<N extends PagedProc> = Omit<ProcArgs<N>, "page" | "q">;
/** The usual filters of a searchable list: the live search text, and whether to ask at all. */
type Q = Pick<ListQueryOptions, "q" | "enabled">;
type O = Pick<ListQueryOptions, "enabled">;
type Opt = keyof ListQueryOptions;

/** The procedure's own filters out of a hook's `f` (the query options stay with `useListQuery`). */
const own = <T extends object>(f: T) => Object.fromEntries(Object.entries(f).filter(([k]) => !["q", "enabled", "pageSize", "getCursor"].includes(k))) as Omit<T, Opt>;

// One typed hook per `*Page` procedure: the picker's / list's ListSource, rows typed as the generated item.

export const useUsersSource = (f: F<"userPage"> & Q = {}) => useListQuery("userPage", own(f), f);
export const useTeamsSource = (q = "", opts: O = {}) => useListQuery("teamPage", {}, { q, ...opts });
export const useTeamMembersSource = (team: string, f: Omit<F<"teamMemberPage">, "team"> & Q = {}) =>
  useListQuery("teamMemberPage", { team, ...own(f) }, f);
export const useCooksSource = (f: F<"cookPage"> & Q = {}) => useListQuery("cookPage", own(f), f);
export const useFiresSource = (q = "") => useListQuery("firePage", {}, { q });
export const useWakeTargetsSource = (cook: string, opts: O = {}) => useListQuery("wakeTargetPage", { cook }, opts);
export const useForgeUnfollowedSource = (q = "") => useListQuery("forgeUnfollowedPage", {}, { q });
export const useStationsSource = (f: F<"stationPage"> & Q = {}) =>
  useListQuery("stationPage", own(f), f);
export const useActivitySource = (project: string, f: Omit<F<"activityPage">, "project"> & O = {}) =>
  useListQuery("activityPage", { project, ...own(f) }, f);
export const useEventsSource = (project: string, f: Omit<F<"eventPage">, "project"> & O = {}) =>
  useListQuery("eventPage", { project, ...own(f) }, f);
export const useFilesSource = (project: string, f: Omit<F<"filePage">, "project"> & Q = {}) =>
  useListQuery("filePage", { project, ...own(f) }, f);
export const useUsageCooksSource = (f: F<"usageCookPage"> & Q = {}) => useListQuery("usageCookPage", own(f), f);
export const usePotsSource = (f: F<"potPage"> & Q = {}) => useListQuery("potPage", own(f), f);
export const useTasksSource = (project: string, f: Omit<F<"taskPage">, "project"> & Q & Pick<ListQueryOptions, "pageSize"> = {}) =>
  useListQuery("taskPage", { project, ...own(f) }, f);
export const useMenusSource = (project: string, f: Omit<F<"menuPage">, "project"> & Q = {}) => useListQuery("menuPage", { project, ...own(f) }, f);
export const useSavedViewsSource = (project: string, q = "") => useListQuery("savedViewPage", { project }, { q });
export const useViewSource = (project: string, f: Omit<F<"viewPage">, "project"> & O = {}) =>
  useListQuery("viewPage", { project, ...own(f) }, f);
export const useLabelsSource = (project: string, f: { q?: string; bucket?: string | null; enabled?: boolean } = {}) =>
  useListQuery("labelPage", { project, ...own(f) }, f);
export const useTagsSource = (project: string, f: { q?: string; kind?: string | null; enabled?: boolean } = {}) =>
  useListQuery("tagPage", { project, ...own(f) }, f);
export const useReposSource = (project: string, q = "") => useListQuery("repoPage", { project }, { q });
export const usePassConversationsSource = (project: string, f: { q?: string; unread?: boolean | null; enabled?: boolean } = {}) =>
  useListQuery("passConversationPage", { project, ...own(f) }, f);
export const useCommentsSource = (task: string, opts: O = {}) => useListQuery("commentPage", { task }, opts);
export const useQuestionsSource = (project: string, f: Omit<F<"questionPage">, "project"> & O = {}) =>
  useListQuery("questionPage", { project, ...own(f) }, f);
export const useMyQuestionsSource = (f: F<"questionMinePage"> & O = {}) => useListQuery("questionMinePage", own(f), f);
export const useReviewsSource = (project: string, f: Omit<F<"reviewPage">, "project"> & O = {}) =>
  useListQuery("reviewPage", { project, ...own(f) }, f);
export const useMyWorkSource = (section: string, f: { project?: string | null; enabled?: boolean } = {}) =>
  useListQuery("myWorkPage", { section, ...own(f) }, f);
