import { useCallback, useEffect, useId, useMemo, type Key, type ReactNode } from "react";
import { Check, ChevronDown, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { useIsPhone } from "@/lib/media";
import { useListbox, type ListSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { useLabelCache } from "@/lib/label-cache";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "./sheet";
import { VirtualListbox } from "./virtual-listbox";

/** What a picker's rows know about themselves when they render. */
export interface PickerRowState {
  index: number;
  selected: boolean;
  active: boolean;
}

export interface PickerProps<T> {
  /** The element that opens it (a button; it gets the ref and the popup attributes). */
  trigger: ReactNode;
  /** The listbox's accessible name (and the sheet's title on phones). */
  label: string;
  source: ListSource<T>;
  /** The search text changed: a server-backed source reads again with it. Without it the rows are filtered here by `filter`. */
  onQuery?: ((q: string) => void) | undefined;
  /** Client-side filter, used when there is no `onQuery`. Without either, the rows are shown whole. */
  filter?: ((item: T, q: string) => boolean) | undefined;
  /** Whether there is a search box (default true). */
  searchable?: boolean | undefined;
  placeholder?: string | undefined;
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T, state: PickerRowState) => ReactNode;
  selected?: ((item: T) => boolean) | undefined;
  multiple?: boolean | undefined;
  /** An option was picked. A single pick closes the popup; a multiple one keeps it open. */
  onPick: (item: T) => void;
  isDisabled?: ((item: T) => boolean) | undefined;
  /** Above the search box. */
  header?: ReactNode;
  empty?: ReactNode;
  defaultOpen?: boolean | undefined;
  open?: boolean | undefined;
  onOpenChange?: ((open: boolean) => void) | undefined;
  align?: "start" | "center" | "end" | undefined;
  /** On the popup (its width follows the trigger's, at least 15rem). */
  className?: string | undefined;
  rows?: number | undefined;
}

/**
 * The popup shared by `VirtualCombobox` and `VirtualMenu`: a Radix popover on desktop, a bottom sheet on phones, with
 * a search input (`role="combobox"`, driving the listbox with `aria-activedescendant`) over a `VirtualListbox`.
 */
export function Picker<T>({
  trigger,
  label,
  source,
  onQuery,
  filter,
  searchable = true,
  placeholder,
  itemKey,
  renderItem,
  selected,
  multiple = false,
  onPick,
  isDisabled,
  header,
  empty,
  defaultOpen = false,
  open: controlledOpen,
  onOpenChange,
  align = "start",
  className,
  rows = 8,
}: PickerProps<T>) {
  const phone = useIsPhone();
  const made = useId();
  const listId = `${made}-list`;
  const inner = useStoreValue(defaultOpen);
  const open = controlledOpen ?? inner.value;
  const query = useStoreValue("");

  const setOpen = useCallback(
    (next: boolean) => {
      inner.set(next);
      if (!next && query.value) {
        query.set("");
        onQuery?.("");
      }
      onOpenChange?.(next);
    },
    [inner, query, onQuery, onOpenChange],
  );
  const setQuery = (q: string) => {
    query.set(q);
    onQuery?.(q);
  };

  const q = query.value;
  const visible = useMemo<ListSource<T>>(() => (onQuery || !filter || !q.trim() ? source : { ...source, items: source.items.filter((it) => filter(it, q)) }), [source, onQuery, filter, q]);

  const pickItem = useCallback(
    (item: T) => {
      onPick(item);
      if (!multiple) setOpen(false);
    },
    [onPick, multiple, setOpen],
  );
  const loadMore = useCallback(() => {
    if (visible.hasNextPage && !visible.isFetchingNextPage) visible.fetchNextPage();
  }, [visible]);
  const lb = useListbox<T>({ items: visible.items, onPick: pickItem, isDisabled, idPrefix: made, pending: visible.pending, resetOn: q, onEscape: () => setOpen(false), fetchMore: loadMore });

  const row = (item: T, s: PickerRowState) => (
    <>
      <span className="min-w-0 flex-1">{renderItem(item, s)}</span>
      {s.selected && <Check className="ml-2 size-4 shrink-0 text-primary" aria-hidden="true" />}
    </>
  );

  const body = (
    <>
      {header}
      {searchable && (
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            role="combobox"
            aria-label={`Search ${label}`}
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={lb.activeId}
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            value={q}
            placeholder={placeholder ?? "Search"}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={lb.onKeyDown}
            className="h-11 min-w-0 flex-1 bg-transparent text-base outline-none placeholder:text-muted-foreground md:h-10 md:text-sm"
          />
        </div>
      )}
      <VirtualListbox<T>
        source={visible}
        itemKey={itemKey}
        renderItem={row}
        label={label}
        selected={selected}
        multiple={multiple}
        isDisabled={isDisabled}
        empty={empty}
        rows={rows}
        id={listId}
        {...(searchable ? { controller: lb } : { onPick: pickItem })}
        className="p-1"
      />
    </>
  );

  if (phone) {
    return (
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetTrigger asChild>{trigger}</SheetTrigger>
        <SheetContent side="bottom" aria-describedby={undefined} className={cn("gap-0 p-0 pb-[env(safe-area-inset-bottom)]", className)}>
          <SheetHeader className="p-4 pb-2">
            <SheetTitle>{label}</SheetTitle>
          </SheetHeader>
          {body}
        </SheetContent>
      </Sheet>
    );
  }
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild aria-haspopup="listbox">
        {trigger}
      </PopoverTrigger>
      <PopoverContent align={align} className={cn("w-(--radix-popover-trigger-width) min-w-60 max-w-[calc(100vw-1rem)] overflow-hidden p-0", className)}>
        {body}
      </PopoverContent>
    </Popover>
  );
}

const NULL_OPTION = Symbol("null option");
type Row<T> = T | typeof NULL_OPTION;

interface ComboboxBase<T> {
  source: ListSource<T>;
  /** The search text changed: read the source again with it. Without it the rows are filtered here by `itemText`. */
  onQuery?: ((q: string) => void) | undefined;
  itemKey: (item: T, index: number) => Key;
  /** The text of an option: what the trigger shows when it is picked, and what client-side search matches. */
  itemText: (item: T) => string;
  /**
   * The text of a value whose row is not in the loaded source (restored from the URL, or the server search moved on):
   * what the trigger shows for it. Without it, the text it had when it was last seen or picked, else the key.
   */
  valueText?: ((key: string) => string | undefined) | undefined;
  /** How a row looks; default `itemText`. */
  renderItem?: ((item: T, state: PickerRowState) => ReactNode) | undefined;
  /** The accessible name, and the sheet's title on phones. */
  label: string;
  placeholder?: string | undefined;
  /** A first option that clears the value, e.g. "Anyone". */
  nullOption?: string | undefined;
  /** How the trigger looks: a form field, or a small pill (a filter chip). */
  trigger?: "field" | "pill" | undefined;
  disabled?: boolean | undefined;
  id?: string | undefined;
  defaultOpen?: boolean | undefined;
  className?: string | undefined;
}
export interface VirtualComboboxSingle<T> extends ComboboxBase<T> {
  multiple?: false | undefined;
  value: string | null;
  onChange: (value: string | null) => void;
}
export interface VirtualComboboxMulti<T> extends ComboboxBase<T> {
  multiple: true;
  value: string[];
  onChange: (value: string[]) => void;
}
export type VirtualComboboxProps<T> = VirtualComboboxSingle<T> | VirtualComboboxMulti<T>;

/**
 * A searchable select over a virtualized, optionally paged list: for data-driven options (people, labels, pots) that
 * may be thousands. A Radix popover on desktop, a bottom sheet on phones. `ui/select.tsx` stays for fixed enums.
 */
export function VirtualCombobox<T>(props: VirtualComboboxProps<T>) {
  const { source, onQuery, itemKey, itemText, valueText, renderItem, label, placeholder, nullOption, trigger = "field", disabled, id, defaultOpen, className } = props;
  const keyOf = useCallback((item: T, index: number) => String(itemKey(item, index)), [itemKey]);
  const chosen: string[] = props.multiple ? props.value : props.value === null ? [] : [props.value];

  const rowsSource = useMemo<ListSource<Row<T>>>(() => (nullOption ? { ...source, items: [NULL_OPTION, ...source.items] } : source), [source, nullOption]);
  const isNull = (r: Row<T>): r is typeof NULL_OPTION => r === NULL_OPTION;

  const cache = useLabelCache();
  // the text of the chosen values is kept as their rows pass by, so the trigger has it once the rows are gone
  const { remember } = cache;
  useEffect(() => {
    const want = new Set(chosen);
    const seen: [string, string][] = [];
    for (let i = 0; i < source.items.length && seen.length < want.size; i++) {
      const it = source.items[i]!;
      const k = String(itemKey(it, i));
      if (want.has(k)) seen.push([k, itemText(it)]);
    }
    if (seen.length) remember(seen);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chosen.join("\u0000"), source.items, itemKey, itemText, remember]);

  const onPick = (r: Row<T>) => {
    if (!isNull(r)) remember([[keyOf(r, 0), itemText(r)]]);
    if (props.multiple) {
      if (isNull(r)) return props.onChange([]);
      const key = keyOf(r, 0);
      props.onChange(chosen.includes(key) ? chosen.filter((k) => k !== key) : [...chosen, key]);
    } else props.onChange(isNull(r) ? null : keyOf(r, 0));
  };
  // a row's key needs its index only for the caller's `itemKey`; the real index comes from the list
  const rowKey = (r: Row<T>, index: number): Key => (isNull(r) ? "\u0000null" : itemKey(r, nullOption ? index - 1 : index));
  const selectedRow = (r: Row<T>) => (isNull(r) ? chosen.length === 0 : chosen.includes(String(itemKey(r, 0))));

  const shown = useMemo(() => {
    if (chosen.length === 0) return null;
    const text = (k: string) => {
      const at = source.items.findIndex((it, i) => String(itemKey(it, i)) === k);
      return at >= 0 ? itemText(source.items[at]!) : (valueText?.(k) ?? cache.labels[k] ?? k);
    };
    return chosen.length === 1 ? text(chosen[0]!) : `${text(chosen[0]!)} +${chosen.length - 1}`;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chosen.join("\u0000"), source.items, itemKey, itemText, valueText, cache.labels]);

  const button = (
    <button
      type="button"
      id={id}
      disabled={disabled}
      aria-label={label}
      className={cn(
        "inline-flex items-center gap-1.5 text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50",
        trigger === "pill"
          ? cn("h-8 rounded-full border px-3 text-[12.5px] transition-colors", shown ? "border-primary/60 bg-primary/10" : "border-input bg-card hover:border-foreground/30")
          : "h-9 w-full justify-between rounded-md border border-input bg-card px-3 shadow-xs data-[state=open]:border-ring",
        className,
      )}
    >
      {trigger === "pill" ? (
        <>
          <span className="text-muted-foreground">{label}</span>
          {shown && <span className="font-medium">{shown}</span>}
        </>
      ) : (
        <span className={cn("truncate", !shown && "text-muted-foreground")}>{shown ?? placeholder ?? label}</span>
      )}
      <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  );

  return (
    <Picker<Row<T>>
      trigger={button}
      label={label}
      source={rowsSource}
      onQuery={onQuery}
      filter={(r, q) => isNull(r) || itemText(r).toLowerCase().includes(q.trim().toLowerCase())}
      itemKey={rowKey}
      renderItem={(r, s) => (isNull(r) ? <span className="text-muted-foreground">{nullOption}</span> : renderItem ? renderItem(r, s) : <span className="truncate">{itemText(r)}</span>)}
      selected={selectedRow}
      multiple={!!props.multiple}
      onPick={onPick}
      placeholder={`Search ${label.toLowerCase()}`}
      defaultOpen={defaultOpen}
    />
  );
}
