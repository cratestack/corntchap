import { useId, useMemo, useRef, type KeyboardEvent } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { matchText, useListbox, useStaticSource, type ListSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { Popover, PopoverAnchor, PopoverContent } from "./ui/popover";
import { VirtualListbox } from "./ui/virtual-listbox";

export interface TagInputProps {
  value: string[];
  onChange: (v: string[]) => void;
  label: string;
  placeholder?: string;
  /** Applied to every tag as it is added (lower-casing a label, upper-casing a task id). */
  normalize?: (s: string) => string;
  /** A short fixed list of options, narrowed here as you type. */
  suggestions?: string[];
  /** Options that may be thousands, paged (`useListQuery`); takes the place of `suggestions`. */
  source?: ListSource<string>;
  /** The text typed changed: a server-backed `source` reads again with it. Without it the source's rows are narrowed here. */
  onQuery?: (q: string) => void;
  /** Whether text that is not an option can be added (default true). When false only options can. */
  allowNew?: boolean;
  /** Focus the field on mount (and so show the options). */
  autoFocus?: boolean;
  className?: string;
}

/**
 * Chips input. Enter, comma or Tab commits: the highlighted option once you have moved to it with the arrow keys,
 * else the typed text (with `allowNew: false`, the first option). Backspace on an empty field removes the last chip.
 * Options (`suggestions` or a paged `source`) show in a virtualized listbox under the field while it has focus.
 */
export function TagInput({ value, onChange, placeholder, suggestions, source, onQuery, allowNew = true, autoFocus = false, normalize = (s) => s, label, className }: TagInputProps) {
  const made = useId();
  const listId = `${made}-list`;
  const root = useRef<HTMLDivElement>(null);
  const text = useStoreValue("");
  const focused = useStoreValue(autoFocus);
  const dismissed = useStoreValue(false);

  const fixed = useStaticSource(suggestions ?? []);
  const base = source ?? fixed;
  const configured = !!(source || suggestions);
  const q = text.value;
  // what is already a chip is not offered again; without `onQuery` the typed text narrows the rows here
  const options = useMemo<ListSource<string>>(() => {
    const chosen = new Set(value);
    return { ...base, items: base.items.filter((s) => !chosen.has(s) && (onQuery && source ? true : matchText(q, s))) };
  }, [base, value, q, onQuery, source]);

  const commit = (raw: string) => {
    const parts = raw
      .split(",")
      .map((s) => normalize(s.trim()))
      .filter(Boolean);
    const next = [...value];
    for (const p of parts) if (!next.includes(p)) next.push(p);
    if (next.length !== value.length) onChange(next);
    setText("");
  };
  const setText = (t: string) => {
    text.set(t);
    dismissed.set(false);
    onQuery?.(t);
  };

  const open = configured && focused.value && !dismissed.value && (options.items.length > 0 || options.isLoading || !!options.error);
  const lb = useListbox<string>({ items: options.items, onPick: (s) => commit(s), idPrefix: made, resetOn: q, onEscape: () => dismissed.set(true) });

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const typed = text.value.trim();
    const option = open && lb.engaged ? options.items[lb.active] : undefined;
    if (e.key === "Enter" || e.key === "Tab") {
      if (option !== undefined) {
        e.preventDefault();
        commit(option);
        return;
      }
      if (typed) {
        const first = open ? options.items[lb.active] : undefined;
        if (allowNew) {
          e.preventDefault();
          commit(typed);
          return;
        }
        if (first !== undefined) {
          e.preventDefault();
          commit(first);
          return;
        }
        if (e.key === "Enter") e.preventDefault();
      }
      return;
    }
    if (e.key === ",") {
      if (typed && (allowNew || options.items.includes(typed))) {
        e.preventDefault();
        commit(typed);
      }
      return;
    }
    if (e.key === "Backspace" && !text.value && value.length) {
      onChange(value.slice(0, -1));
      return;
    }
    if (!open) {
      if ((e.key === "ArrowDown" || e.key === "ArrowUp") && configured) {
        e.preventDefault();
        dismissed.set(false);
      }
      return;
    }
    lb.onKeyDown(e);
  };

  return (
    <Popover open={open} onOpenChange={(o) => !o && dismissed.set(true)}>
      <PopoverAnchor asChild>
        <div
          ref={root}
          className={cn(
            "flex min-h-9 flex-wrap items-center gap-1.5 rounded-md border border-input bg-card px-2 py-1 shadow-xs focus-within:ring-[3px] focus-within:ring-ring/50",
            className,
          )}
        >
          {value.map((v) => (
            <span key={v} className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-[12px] font-medium">
              {v}
              <button
                type="button"
                aria-label={`Remove ${v}`}
                className="rounded-full text-muted-foreground hover:text-foreground"
                onClick={() => onChange(value.filter((x) => x !== v))}
              >
                <X className="size-3" />
              </button>
            </span>
          ))}
          <input
            role="combobox"
            aria-label={label}
            aria-expanded={open}
            aria-controls={configured ? listId : undefined}
            aria-autocomplete={configured ? "list" : "none"}
            aria-haspopup={configured ? "listbox" : undefined}
            aria-activedescendant={open ? lb.activeId : undefined}
            autoFocus={autoFocus}
            autoComplete="off"
            value={q}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onKey}
            onFocus={() => focused.set(true)}
            onBlur={() => {
              focused.set(false);
              if (allowNew) commit(text.value);
            }}
            placeholder={value.length ? "" : placeholder}
            className="min-w-24 flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-trigger-width) min-w-56 p-1"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        onInteractOutside={(e) => {
          if (root.current?.contains(e.target as Node)) e.preventDefault();
        }}
        // keep focus in the field while an option is clicked, or the blur would add the half-typed text
        onMouseDown={(e) => e.preventDefault()}
      >
        <VirtualListbox<string>
          source={options}
          itemKey={(s) => s}
          renderItem={(s) => <span className="truncate">{s}</span>}
          label={`${label} options`}
          id={listId}
          controller={lb}
          empty="No options."
        />
      </PopoverContent>
    </Popover>
  );
}
