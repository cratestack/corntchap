import { useCallback, useId, type Key, type KeyboardEvent, type ReactNode } from "react";
import type { ListSource } from "@/lib/listbox";
import { useListbox } from "@/lib/listbox";
import { Popover, PopoverAnchor, PopoverContent } from "./ui/popover";
import { VirtualListbox } from "./ui/virtual-listbox";

/** An `@name` being typed at the caret: where the `@` is, and the text after it (lower-cased). */
export interface Mention {
  start: number;
  q: string;
}

/** The mention the caret is in the middle of, if any: an `@` at the start of the text or after whitespace, then word characters up to the caret. */
export function parseMention(text: string, caret: number): Mention | null {
  const m = /(^|\s)@([\w.-]*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2]!.length - 1, q: m[2]!.toLowerCase() } : null;
}

/** The text with the mention replaced by `@name ` and where the caret goes after it. */
export function insertMention(text: string, caret: number, mention: Mention, name: string): { text: string; caret: number } {
  return { text: `${text.slice(0, mention.start)}@${name} ${text.slice(caret)}`, caret: mention.start + name.length + 2 };
}

/** What the textarea needs to be the popup's combobox. */
export interface MentionBind {
  role: "combobox";
  "aria-autocomplete": "list";
  "aria-haspopup": "listbox";
  "aria-expanded": boolean;
  "aria-controls": string;
  "aria-activedescendant": string | undefined;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}

export interface MentionPopupProps<T> {
  /** The textarea (or input) the popup hangs on: spread `bind` onto it. It is the popup's anchor and keeps focus. */
  anchor: (bind: MentionBind) => ReactNode;
  /** A mention is being typed (`parseMention` found one). */
  open: boolean;
  /** The text after the `@` (the active row goes back to the first when it changes). */
  query: string;
  source: ListSource<T>;
  itemKey: (item: T, index: number) => Key;
  renderItem: (item: T) => ReactNode;
  onPick: (item: T) => void;
  /** Escape was pressed. */
  onClose: () => void;
  label?: string;
}

/**
 * The options under an `@` in a message box: a virtualized listbox above the textarea (`side="top"`), driven by the
 * textarea's keys (arrows, Home/End, Enter or Tab to insert, Escape to close) while focus stays in it. The caller
 * parses the mention (`parseMention`), reads the source with its text and inserts the pick (`insertMention`).
 */
export function MentionPopup<T>({ anchor, open, query, source, itemKey, renderItem, onPick, onClose, label = "Mention a chef" }: MentionPopupProps<T>) {
  const made = useId();
  const listId = `${made}-list`;
  const shown = open && (source.items.length > 0 || source.isLoading || !!source.error);
  const loadMore = useCallback(() => {
    if (source.hasNextPage && !source.isFetchingNextPage) source.fetchNextPage();
  }, [source]);
  const lb = useListbox<T>({ items: source.items, onPick, idPrefix: made, onEscape: onClose, fetchMore: loadMore, resetOn: query });

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (!shown) return;
    if (e.key === "Tab" && !e.shiftKey && source.items.length > 0) {
      e.preventDefault();
      lb.pick(lb.active);
      return;
    }
    lb.onKeyDown(e);
  };

  const bind: MentionBind = {
    role: "combobox",
    "aria-autocomplete": "list",
    "aria-haspopup": "listbox",
    "aria-expanded": shown,
    "aria-controls": listId,
    "aria-activedescendant": shown ? lb.activeId : undefined,
    onKeyDown,
  };

  return (
    <Popover open={shown} onOpenChange={(o) => !o && onClose()}>
      <PopoverAnchor asChild>{anchor(bind)}</PopoverAnchor>
      <PopoverContent
        side="top"
        align="start"
        className="w-80 max-w-[calc(100vw-1.25rem)] p-1"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        onInteractOutside={(e) => {
          // a click in the textarea moves the caret; it does not close the popup
          if ((e.target as HTMLElement).closest("[aria-controls]")?.getAttribute("aria-controls") === listId) e.preventDefault();
        }}
        onMouseDown={(e) => e.preventDefault()}
      >
        <VirtualListbox<T> source={source} itemKey={itemKey} renderItem={renderItem} label={label} id={listId} controller={lb} rows={6} />
      </PopoverContent>
    </Popover>
  );
}
