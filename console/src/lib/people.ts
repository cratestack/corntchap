import { useEffect, useMemo } from "react";
import type { PrincipalView, TeamView, UserView } from "@corntchap/api";
import { cookState, type CookState } from "@/components/bits";
import type { FacetDef, FacetOption } from "@/components/filter-bar";
import { staticSource, useMappedSource, type ListSource } from "./listbox";
import { useProc } from "./data";
import { useCooksSource, useFiresSource, useTeamsSource, useUsersSource } from "./sources";
import { useStoreValue } from "./use-store-value";

/**
 * The row named `name` (any case) out of a source searched for that name: while it is not among the rows loaded and
 * there is a next page, the next page is read, so a name that is a prefix of many others is still found.
 */
function useNamed<T extends { name: string }>(source: ListSource<T>, name: string) {
  const lower = name.toLowerCase();
  const item = useMemo(() => source.items.find((r) => r.name.toLowerCase() === lower), [source.items, lower]);
  const { hasNextPage, isFetchingNextPage, pending, fetchNextPage } = source;
  useEffect(() => {
    if (!item && hasNextPage && !isFetchingNextPage && !pending) fetchNextPage();
  }, [item, hasNextPage, isFetchingNextPage, pending, fetchNextPage]);
  const searching = !item && (hasNextPage || !!pending);
  return { item, isLoading: source.isLoading || searching, error: source.error };
}

/** One cook by name (for its page), from the cook list searched by that name. */
export function useCookByName(name: string, enabled = true) {
  return useNamed(useCooksSource({ q: name, enabled: enabled && !!name }), name);
}

/** One fire by name. */
export function useFireByName(name: string) {
  return useNamed(useFiresSource(name), name);
}

/** One team by name (with the signed-in person's role in it). */
export function useTeamByName(name: string | null | undefined) {
  return useNamed(useTeamsSource(name ?? "", { enabled: !!name }), name ?? "");
}

/** A cook's or fire's state at `now` (from its list row). */
export function principalState(p: Pick<PrincipalView, "disabled" | "busy" | "last_seen_at">, now: number): CookState {
  return cookState({ disabled: p.disabled, busy: p.busy, last_seen_at: p.last_seen_at ?? null }, now);
}

/**
 * The fires in one line for the sidebar and the More sheet, from two small reads whatever the number of fires: the lit
 * ones (the first, by `firePage`'s `lit` filter; `more` when there are further ones) and the summary (how many fires,
 * and when one was last seen).
 */
export function useFireSummary() {
  const lit = useFiresSource("", { lit: true }, { pageSize: 1 });
  const summary = useProc("fireSummary", {});
  return { lit: lit.items, more: !!lit.hasNextPage, total: summary.data?.total ?? 0, newest: summary.data?.last_seen_at ?? null, isLoading: lit.isLoading || summary.isLoading };
}

const teamOption = (t: TeamView): FacetOption => ({ value: t.name, label: t.name, hint: `${t.members} ${t.members === 1 ? "chef" : "chefs"}` });
const userOption = (u: UserView): FacetOption => ({ value: u.name, label: u.display_name ? `${u.display_name} @${u.name}` : u.name });

/** A "Team" facet for a `FilterBar`: the teams, paged and searched by the server. */
export function useTeamFacet(id = "team", label = "Team"): FacetDef {
  const q = useStoreValue("");
  const source = useMappedSource(useTeamsSource(q.value), teamOption);
  return { id, label, source, onQuery: q.set };
}

/** An "Owner" facet: the people, paged and searched by the server (`enabled`: only orchestrators and super-admins read the directory). */
export function usePersonFacet(id = "owner", label = "Owner", enabled = true): FacetDef {
  const q = useStoreValue("");
  const source = useMappedSource(useUsersSource({ q: q.value, enabled }), userOption);
  return { id, label, source, onQuery: q.set, disabled: !enabled };
}

/** A facet of fixed options (a role, enabled or disabled). */
export function staticFacet(id: string, label: string, options: readonly FacetOption[]): FacetDef {
  return { id, label, source: staticSource(options), searchable: false };
}

/** The "Disabled" facet's options; read the facet's value with {@link disabledArg}. */
export const DISABLED_OPTIONS: readonly FacetOption[] = [
  { value: "true", label: "Disabled" },
  { value: "false", label: "Enabled" },
];

/** A facet value as a procedure's `disabled` argument (`"true"` / `"false"` / none). */
export const disabledArg = (v: string): { disabled: boolean } | Record<string, never> => (v === "true" ? { disabled: true } : v === "false" ? { disabled: false } : {});
