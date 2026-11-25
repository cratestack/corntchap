import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router";

/** One filter in the URL: `string` (`?owner=ada`), `list` (`?label=a,b`, comma-joined) or `bool` (`?mine=1`). */
export type UrlFilter =
  | { type: "string"; param?: string; default?: string }
  | { type: "list"; param?: string; default?: string[] }
  | { type: "bool"; param?: string; default?: boolean };

export type UrlFilterSchema = Record<string, UrlFilter>;

export type UrlFilterValues<S extends UrlFilterSchema> = {
  [K in keyof S]: S[K] extends { type: "list" } ? string[] : S[K] extends { type: "bool" } ? boolean : string;
};

/** The name of the search text's parameter. */
export const Q_PARAM = "q";

const sameList = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * A page's filters held in the URL, so a filtered list can be linked, reloaded and navigated back to. Reads the
 * search params through `schema`; `set(patch)` changes some of them (a value equal to its default drops the
 * param), `clear()` drops them all (and the search text), `q` / `setQ` are the search text. Every change replaces
 * the history entry (typing a search is not a page to go back through).
 */
export function useUrlFilters<S extends UrlFilterSchema>(schema: S) {
  const [params, setParams] = useSearchParams();

  const values = useMemo(() => {
    const out: Record<string, string | string[] | boolean> = {};
    for (const [key, def] of Object.entries(schema)) {
      const raw = params.get(def.param ?? key);
      if (def.type === "list") out[key] = raw === null ? (def.default ?? []) : raw.split(",").filter(Boolean);
      else if (def.type === "bool") out[key] = raw === null ? (def.default ?? false) : raw === "1" || raw === "true";
      else out[key] = raw ?? def.default ?? "";
    }
    return out as UrlFilterValues<S>;
    // the schema is a constant of the page
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  const set = useCallback(
    (patch: Partial<UrlFilterValues<S>>) =>
      setParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          for (const [key, value] of Object.entries(patch)) {
            const def = schema[key];
            if (!def || value === undefined) continue;
            const name = def.param ?? key;
            if (def.type === "list") {
              const list = value as string[];
              if (list.length === 0 || sameList(list, def.default ?? [])) next.delete(name);
              else next.set(name, list.join(","));
            } else if (def.type === "bool") {
              if (value === (def.default ?? false)) next.delete(name);
              else next.set(name, value ? "1" : "0");
            } else if (value === "" || value === (def.default ?? "")) next.delete(name);
            else next.set(name, value as string);
          }
          return next;
        },
        { replace: true },
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [setParams],
  );

  const clear = useCallback(
    () =>
      setParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          next.delete(Q_PARAM);
          for (const [key, def] of Object.entries(schema)) next.delete(def.param ?? key);
          return next;
        },
        { replace: true },
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [setParams],
  );

  const setQ = useCallback(
    (q: string) =>
      setParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (q) next.set(Q_PARAM, q);
          else next.delete(Q_PARAM);
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );

  const q = params.get(Q_PARAM) ?? "";
  /** How many filters (not counting the search text) are off their defaults. */
  const active = useMemo(() => Object.keys(schema).filter((k) => params.has(schema[k]!.param ?? k)).length, [params, schema]);
  return { values, set, clear, q, setQ, active };
}
