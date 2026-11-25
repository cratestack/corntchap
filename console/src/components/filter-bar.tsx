import { useEffect, type ReactNode, type Ref } from "react";
import { ChevronDown, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ListSource } from "@/lib/listbox";
import { useLabelCache } from "@/lib/label-cache";
import { VirtualMenu, type VirtualMenuProps } from "./ui/virtual-menu";

export interface FacetOption {
  value: string;
  label: string;
  /** Small text after the label (a count, a kind). */
  hint?: string | undefined;
}

export interface FacetDef {
  /** The key in `values`. */
  id: string;
  label: string;
  icon?: ReactNode;
  source: ListSource<FacetOption>;
  /** The search text typed in the facet's menu, for a server-backed source; without it the options are filtered here. */
  onQuery?: ((q: string) => void) | undefined;
  /**
   * The label of a value whose option is not in the loaded source (it came back from the URL, or the server search
   * moved on). Without it the chip shows the label the value had when it was last seen or picked, else the value.
   */
  valueText?: ((value: string) => string | undefined) | undefined;
  /** Several values (`values[id]` is a list); default one. */
  multiple?: boolean | undefined;
  /** Whether the menu has a search box (default true). */
  searchable?: boolean | undefined;
  disabled?: boolean | undefined;
}

export type FacetValue = string | string[] | undefined;

interface ChipProps<T> extends Omit<VirtualMenuProps<T>, "trigger"> {
  icon?: ReactNode;
  /** What is chosen, shown on the chip; undefined when nothing is (the chip is plain, with no clear button). */
  value?: string | undefined;
  onClear: () => void;
  disabled?: boolean | undefined;
}

/** A filter chip: a pill that opens a searchable, virtualized menu of the facet's options, with an X to clear it. */
export function FilterChip<T>({ icon, value, onClear, disabled, label, ...menu }: ChipProps<T>) {
  return (
    <div
      className={cn(
        "inline-flex h-8 items-center rounded-full border text-[12.5px] transition-colors",
        value ? "border-primary/60 bg-primary/10" : "border-input bg-card hover:border-foreground/30",
        disabled && "opacity-50",
      )}
    >
      <VirtualMenu<T>
        {...(menu as Omit<VirtualMenuProps<T>, "trigger" | "label">)}
        label={label}
        trigger={
          <button type="button" disabled={disabled} aria-label={value ? `${label}: ${value}` : label} className="inline-flex h-full items-center gap-1.5 rounded-full pr-2 pl-3 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50">
            {icon}
            <span className="text-muted-foreground">{label}</span>
            {value && <span className="max-w-40 truncate font-medium">{value}</span>}
            <ChevronDown className="size-3 text-muted-foreground" aria-hidden="true" />
          </button>
        }
      />
      {value && (
        <button type="button" onClick={onClear} aria-label={`Clear ${label} filter`} className="mr-1.5 rounded-full p-0.5 text-muted-foreground hover:text-foreground">
          <X className="size-3" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

export interface FilterBarProps {
  /** The search text as typed (the list's query debounces it). */
  q: string;
  onQ: (q: string) => void;
  facets: readonly FacetDef[];
  values: Record<string, FacetValue>;
  /** Some facets changed: the new values by facet id (a list for a multiple facet, "" for none). */
  onChange: (patch: Record<string, string | string[]>) => void;
  /** Drop the search text and every facet. */
  onClear: () => void;
  searchPlaceholder?: string | undefined;
  /** No search box (a list with no text to search): just the chips and "Clear filters". */
  hideSearch?: boolean | undefined;
  /** The search box's element, to focus it from outside. */
  searchRef?: Ref<HTMLInputElement> | undefined;
  className?: string | undefined;
}

const toList = (v: FacetValue): string[] => (Array.isArray(v) ? v : v ? [v] : []);

/** A list page's filters: a search box and one chip per facet. The values live with the caller (`useUrlFilters`). */
export function FilterBar({ q, onQ, facets, values, onChange, onClear, searchPlaceholder = "Search", hideSearch = false, searchRef, className }: FilterBarProps) {
  const anyActive = !!q.trim() || facets.some((f) => toList(values[f.id]).length > 0);
  return (
    <div role="search" className={cn("flex flex-wrap items-center gap-2", className)}>
      {!hideSearch && (
      <label className="relative min-w-48 flex-1 basis-56 md:max-w-xs">
        <span className="sr-only">{searchPlaceholder}</span>
        <Search className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
        <input
          ref={searchRef}
          type="search"
          value={q}
          onChange={(e) => onQ(e.target.value)}
          placeholder={searchPlaceholder}
          autoComplete="off"
          className="h-8 w-full rounded-full border border-input bg-card pr-3 pl-8 text-base outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 md:text-[13px]"
        />
      </label>
      )}
      {facets.map((f) => (
        <Facet key={f.id} f={f} chosen={toList(values[f.id])} onChange={onChange} />
      ))}
      {anyActive && (
        <button type="button" onClick={onClear} className="h-8 rounded-full px-2.5 text-[12.5px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
          Clear filters
        </button>
      )}
    </div>
  );
}

/** One facet's chip: it keeps the labels of what was picked, so the chip still names them when their rows are gone. */
function Facet({ f, chosen, onChange }: { f: FacetDef; chosen: string[]; onChange: FilterBarProps["onChange"] }) {
  const { labels, remember } = useLabelCache();
  const { items } = f.source;
  useEffect(() => {
    const want = new Set(chosen);
    const seen: [string, string][] = [];
    for (let i = 0; i < items.length && seen.length < want.size; i++) if (want.has(items[i]!.value)) seen.push([items[i]!.value, items[i]!.label]);
    if (seen.length) remember(seen);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chosen.join("\u0000"), items, remember]);
  const text = (v: string) => items.find((o) => o.value === v)?.label ?? f.valueText?.(v) ?? labels[v] ?? v;
  const shown = chosen.length === 0 ? undefined : chosen.length === 1 ? text(chosen[0]!) : `${text(chosen[0]!)} +${chosen.length - 1}`;
  return (
    <FilterChip<FacetOption>
      label={f.label}
      icon={f.icon}
      value={shown}
      disabled={f.disabled}
      onClear={() => onChange({ [f.id]: f.multiple ? [] : "" })}
      source={f.source}
      onQuery={f.onQuery}
      itemText={(o) => `${o.label} ${o.hint ?? ""}`}
      itemKey={(o) => o.value}
      searchable={f.searchable}
      mode={f.multiple ? "multi" : "single"}
      selected={(o) => chosen.includes(o.value)}
      onSelect={(o) => {
        remember([[o.value, o.label]]);
        onChange({ [f.id]: f.multiple ? (chosen.includes(o.value) ? chosen.filter((v) => v !== o.value) : [...chosen, o.value]) : chosen.includes(o.value) ? "" : o.value });
      }}
      renderItem={(o) => (
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="truncate">{o.label}</span>
          {o.hint && <span className="shrink-0 text-[12px] text-muted-foreground">{o.hint}</span>}
        </span>
      )}
    />
  );
}
