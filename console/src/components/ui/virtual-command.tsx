import { useCallback, useId, useMemo, type ReactNode } from "react";
import { GroupedVirtuoso } from "react-virtuoso";
import { Loader2, Search } from "lucide-react";
import { matchText, useListbox, type ListSource } from "@/lib/listbox";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "./dialog";
import { listboxComponents, type ListboxBinding, type VirtualContext } from "./virtual-shared";

export interface CommandItem {
  id: string;
  label: string;
  /** Extra words client-side search matches. */
  keywords?: string[] | undefined;
  icon?: ReactNode;
  /** Small text at the end of the row (a shortcut, a count). */
  hint?: ReactNode;
  onSelect: () => void;
}

export interface CommandSection {
  id: string;
  heading: string;
  source: ListSource<CommandItem>;
  /** `"client"`: the rows are narrowed here by the query; `"server"`: the source already is (it was told the query). */
  filter: "client" | "server";
}

export interface VirtualCommandProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string | undefined;
  placeholder?: string | undefined;
  query: string;
  onQuery: (q: string) => void;
  sections: readonly CommandSection[];
  /** Shown when no section has a row. */
  empty?: ReactNode;
}

/**
 * A command palette over sections of rows that may be thousands long and paged: one virtualized listbox with sticky
 * section headings and a single active row across them (Arrow keys, Home/End, Enter). A dialog with its own scroller.
 * Replaces `ui/command.tsx` (cmdk), which renders every row.
 */
export function VirtualCommand({ open, onOpenChange, title, description, placeholder, query, onQuery, sections, empty }: VirtualCommandProps) {
  const made = useId();
  const listId = `${made}-list`;

  const shown = useMemo(
    () =>
      sections
        .map((s) => ({ section: s, items: s.filter === "client" ? s.source.items.filter((it) => matchText(query, it.label, ...(it.keywords ?? []))) : s.source.items }))
        .filter((s) => s.items.length > 0),
    [sections, query],
  );
  const flat = useMemo(() => shown.flatMap((s) => s.items), [shown]);
  const groups = useMemo(() => shown.map((s) => s.items.length), [shown]);
  // Virtuoso keys by position in the list, headings included: a section's heading, then its rows
  const keys = useMemo(() => shown.flatMap((s) => [`section:${s.section.id}`, ...s.items.map((it) => `${s.section.id}:${it.id}`)]), [shown]);
  const loading = sections.some((s) => s.source.isLoading);
  const fetching = sections.some((s) => s.source.isFetchingNextPage);
  const failed = sections.find((s) => !!s.source.error && s.source.items.length === 0)?.source.error;

  // the end of the list is the last section with rows: it loads the next page (else any section that has one)
  const loadMore = useCallback(() => {
    const last = shown[shown.length - 1]?.section.source;
    const next = last?.hasNextPage ? last : sections.map((s) => s.source).find((src) => src.hasNextPage);
    if (next && !next.isFetchingNextPage) next.fetchNextPage();
  }, [shown, sections]);

  const pick = useCallback(
    (item: CommandItem) => {
      onOpenChange(false);
      item.onSelect();
    },
    [onOpenChange],
  );
  const lb = useListbox<CommandItem>({ items: flat, onPick: pick, idPrefix: made, resetOn: query, onEscape: () => onOpenChange(false), fetchMore: loadMore });

  const binding = useMemo<ListboxBinding>(
    () => ({ id: listId, multiple: false, activeIndex: lb.active, optionId: lb.optionId, isSelected: () => false, isDisabled: () => false, pick: lb.pick, hover: (i) => lb.setActive(i, { scroll: false }) }),
    [listId, lb],
  );
  const footer = useMemo(
    () =>
      fetching ? (
        <p role="status" className="flex items-center justify-center gap-2 py-2 text-[12.5px] text-muted-foreground">
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
        </p>
      ) : null,
    [fetching],
  );
  const context = useMemo<VirtualContext>(() => ({ label: title, footer, listbox: binding, itemClassName: "mx-1 flex min-h-11 items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm md:min-h-10", stickyClassName: "bg-popover" }), [title, footer, binding]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="top-[12%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl max-sm:top-0 max-sm:h-dvh max-sm:max-h-dvh max-sm:rounded-none max-sm:border-0 max-sm:pt-[env(safe-area-inset-top)] max-sm:pb-[env(safe-area-inset-bottom)] grid-rows-[auto_minmax(0,1fr)]"
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description ?? "Type to search, arrow keys to move, Enter to choose."}</DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-2 border-b border-border px-3.5">
          <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            role="combobox"
            aria-label={title}
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={lb.activeId}
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            value={query}
            placeholder={placeholder ?? "Search"}
            onChange={(e) => onQuery(e.target.value)}
            onKeyDown={lb.onKeyDown}
            className="h-12 min-w-0 flex-1 bg-transparent text-base outline-none placeholder:text-muted-foreground md:text-sm"
          />
        </div>
        {flat.length === 0 ? (
          failed ? (
            <p role="alert" className="p-6 text-center text-sm text-destructive">
              {failed instanceof Error ? failed.message : "Could not load the list."}
            </p>
          ) : loading || fetching ? (
            <p role="status" aria-busy="true" className="p-6 text-center text-sm text-muted-foreground">
              Searching
            </p>
          ) : (
            <p className="p-6 text-center text-sm text-muted-foreground">{empty ?? "Nothing matches that."}</p>
          )
        ) : (
          <div className="min-h-0 py-1 sm:h-[min(26rem,60dvh)] max-sm:h-full">
            <GroupedVirtuoso<CommandItem, VirtualContext>
              ref={lb.listRef as never}
              groupCounts={groups}
              context={context}
              components={listboxComponents}
              computeItemKey={(position) => keys[position] ?? position}
              groupContent={(gi) => <p className="label-caps bg-popover px-3.5 pt-2 pb-1">{shown[gi]?.section.heading}</p>}
              itemContent={(index) => {
                const it = flat[index];
                return it ? (
                  <>
                    {it.icon && <span className="flex shrink-0 items-center text-muted-foreground [&_svg]:size-4">{it.icon}</span>}
                    <span className="min-w-0 flex-1 truncate">{it.label}</span>
                    {it.hint && <span className="shrink-0 font-mono text-[11px] text-muted-foreground">{it.hint}</span>}
                  </>
                ) : null;
              }}
              defaultItemHeight={40}
              increaseViewportBy={{ top: 200, bottom: 400 }}
              endReached={loadMore}
              className="scrollbar-thin overscroll-contain"
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
