import { useMemo } from "react";
import type { MemberView, MenuView, ProjectSummary } from "@corntchap/api";
import { GitBranch } from "lucide-react";
import { matchText, useStaticSource, type ListSource } from "@/lib/listbox";
import { useLabelsSource, useMenusSource, useReposSource, useTeamMembersSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { BUCKETS } from "@/lib/format";
import type { FacetDef, FacetOption } from "./filter-bar";
import { menuOption } from "./menus";

/** The filters of a pot's task views, in the URL: `?status=ready&label=urgent&owner=me&q=retry`. */
export const TASK_FILTERS = {
  menu: { type: "string" },
  status: { type: "string" },
  label: { type: "string" },
  repo: { type: "string" },
  owner: { type: "string" },
  holder: { type: "string" },
} as const;

const NO_MENU: FacetOption = { value: "-", label: "No menu" };
const ME: FacetOption = { value: "me", label: "Me" };
const NOBODY: FacetOption = { value: "-", label: "Nobody" };

const STATUS_OPTIONS: FacetOption[] = BUCKETS.map((b) => ({ value: b.id, label: b.label }));

const labelOption = (l: { label: string; count: number }): FacetOption => ({ value: l.label, label: l.label, hint: String(l.count) });
const repoOption = (r: { name: string }): FacetOption => ({ value: r.name, label: r.name });
const memberOption = (m: MemberView): FacetOption => ({ value: m.name, label: m.display_name ? `${m.display_name} · @${m.name}` : m.name, hint: m.kind === "user" ? "person" : "cook" });

/** A server-backed source as facet options, with a few fixed ones in front (kept while they match the typed text). */
function useOptions<T>(source: ListSource<T>, map: (item: T) => FacetOption, lead: readonly FacetOption[], q: string): ListSource<FacetOption> {
  return useMemo(
    () => ({ ...source, items: [...lead.filter((o) => matchText(q, o.label, o.value)), ...source.items.map(map)] }),
    // `lead` is a module constant at every call
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [source, map, q],
  );
}

const NONE: readonly FacetOption[] = [];
const OWNER_LEAD: readonly FacetOption[] = [ME, NOBODY];
const MENU_LEAD: readonly FacetOption[] = [NO_MENU];

/**
 * The facets of a pot's task views: menu, status, label, repository, owner and cook. Their options come from the
 * pot's own paged reads (menus, labels, repositories, team members) as the chip's menu is searched and scrolled, not
 * from the tasks loaded.
 */
export function useTaskFacets(project: ProjectSummary | undefined, selectedMenu?: MenuView | undefined): FacetDef[] {
  const pot = project?.key ?? "";
  const team = project?.team ?? "";
  const on = !!project;
  const menuQ = useStoreValue("");
  const labelQ = useStoreValue("");
  const repoQ = useStoreValue("");
  const ownerQ = useStoreValue("");
  const holderQ = useStoreValue("");

  const menus = useOptions<MenuView>(useMenusSource(pot, { q: menuQ.value, status: "all", enabled: on }), menuOption, MENU_LEAD, menuQ.value);
  const labels = useOptions(useLabelsSource(pot, { q: labelQ.value, bucket: "all", enabled: on }), labelOption, NONE, labelQ.value);
  const repos = useOptions(useReposSource(pot, repoQ.value), repoOption, NONE, repoQ.value);
  const owners = useOptions(useTeamMembersSource(team, { q: ownerQ.value, enabled: on && !!team }), memberOption, OWNER_LEAD, ownerQ.value);
  const holders = useOptions(useTeamMembersSource(team, { kind: "agent", q: holderQ.value, enabled: on && !!team }), memberOption, NONE, holderQ.value);
  const statuses = useStaticSource(STATUS_OPTIONS);

  const { set: setMenuQ } = menuQ;
  const { set: setLabelQ } = labelQ;
  const { set: setRepoQ } = repoQ;
  const { set: setOwnerQ } = ownerQ;
  const { set: setHolderQ } = holderQ;
  return useMemo<FacetDef[]>(
    () => [
      { id: "menu", label: "Menu", source: menus, onQuery: setMenuQ, valueText: (v) => (v === selectedMenu?.key ? selectedMenu.name : undefined) },
      { id: "status", label: "Status", source: statuses, searchable: false },
      { id: "label", label: "Label", source: labels, onQuery: setLabelQ },
      { id: "repo", label: "Repository", icon: <GitBranch className="size-3 text-primary" aria-hidden="true" />, source: repos, onQuery: setRepoQ },
      { id: "owner", label: "Owner", source: owners, onQuery: setOwnerQ },
      { id: "holder", label: "Cook", source: holders, onQuery: setHolderQ },
    ],
    [menus, statuses, labels, repos, owners, holders, selectedMenu, setMenuQ, setLabelQ, setRepoQ, setOwnerQ, setHolderQ],
  );
}


const actorOption = (m: MemberView): FacetOption => memberOption(m);

/** The facet of a pot's activity feed that narrows it to one actor: the pot's team members, searched and paged. */
export function useActorFacet(project: ProjectSummary | undefined): FacetDef {
  const team = project?.team ?? "";
  const q = useStoreValue("");
  const actors = useOptions(useTeamMembersSource(team, { q: q.value, enabled: !!project && !!team }), actorOption, NONE, q.value);
  const { set } = q;
  return useMemo<FacetDef>(() => ({ id: "actor", label: "Actor", source: actors, onQuery: set }), [actors, set]);
}
