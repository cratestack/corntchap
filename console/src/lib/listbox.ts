import { useCallback, useEffect, useMemo, useRef, type KeyboardEvent, type RefObject } from "react";
import { useStoreValue } from "./use-store-value";

/**
 * Where a picker's options come from: a static array, or a paged read (`useListQuery`). Everything that lists
 * options (VirtualListbox, VirtualCombobox, VirtualMenu, VirtualCommand, TagInput, MentionPopup) takes one.
 */
export interface ListSource<T> {
  items: readonly T[];
  hasNextPage: boolean;
  fetchNextPage: () => void;
  isLoading: boolean;
  isFetchingNextPage: boolean;
  error: unknown;
  /** The most pages the list holds are loaded: refine the search to see more. */
  capped?: boolean | undefined;
  total?: number | null | undefined;
}

/** What the active row is scrolled into view through: a Virtuoso / GroupedVirtuoso handle. */
export interface ScrollsIntoView {
  scrollIntoView: (location: { index: number; behavior?: "auto" | "smooth" }) => void;
}

const noop = () => {};

/** A source over a plain array: nothing to load. */
export function staticSource<T>(items: readonly T[]): ListSource<T> {
  return { items, hasNextPage: false, fetchNextPage: noop, isLoading: false, isFetchingNextPage: false, error: null, total: items.length };
}

/** `staticSource`, kept stable while `items` is. */
export function useStaticSource<T>(items: readonly T[]): ListSource<T> {
  return useMemo(() => staticSource(items), [items]);
}

/** Whether any of `fields` contains `q`, ignoring case; an empty `q` matches everything. */
export function matchText(q: string, ...fields: (string | null | undefined)[]): boolean {
  const needle = q.trim().toLowerCase();
  if (!needle) return true;
  return fields.some((f) => !!f && f.toLowerCase().includes(needle));
}

/** Rows a PageUp / PageDown moves by. */
export const PAGE_STEP = 8;

export interface ListboxOptions<T> {
  items: readonly T[];
  onPick: (item: T, index: number) => void;
  isDisabled?: ((item: T) => boolean) | undefined;
  /** Makes the option ids (`${idPrefix}-opt-${index}`); unique per page (use `useId`). */
  idPrefix: string;
  /** The active row goes back to the first when this changes (the query, say). */
  resetOn?: unknown;
  /** Escape was pressed (the owner closes the popup). */
  onEscape?: (() => void) | undefined;
  /** The active row reached the last loaded one: load more. */
  fetchMore?: (() => void) | undefined;
}

export interface ListboxController {
  /** Index of the active row (the one `aria-activedescendant` names), clamped to the rows there are. */
  active: number;
  /** The user moved the active row with the keys (not just the default first row). */
  engaged: boolean;
  setActive: (index: number, opts?: { scroll?: boolean }) => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
  optionId: (index: number) => string;
  /** The id to give `aria-activedescendant`; undefined when there are no rows. */
  activeId: string | undefined;
  /** Hand to the list's `ref`: the active row is scrolled into view through it. */
  listRef: RefObject<ScrollsIntoView | null>;
  pick: (index: number) => void;
  count: number;
}

/**
 * The keyboard and active-row logic of a listbox, apart from how it looks: ArrowUp / ArrowDown, Home / End, PageUp /
 * PageDown move the active row (skipping disabled ones; it is scrolled into view, even rows not rendered yet),
 * Enter picks it, Escape calls `onEscape`. The element that has focus (the search input, or the listbox itself) gets
 * `onKeyDown` and `aria-activedescendant={activeId}`; focus stays there while the options are virtualized.
 */
export function useListbox<T>({ items, onPick, isDisabled, idPrefix, resetOn, onEscape, fetchMore }: ListboxOptions<T>): ListboxController {
  const state = useStoreValue({ active: 0, engaged: false });
  const listRef = useRef<ScrollsIntoView | null>(null);
  const count = items.length;
  const active = count === 0 ? 0 : Math.min(state.value.active, count - 1);

  useEffect(() => {
    state.set({ active: 0, engaged: false });
    // only a change of `resetOn` resets
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetOn]);

  const optionId = useCallback((index: number) => `${idPrefix}-opt-${index}`, [idPrefix]);

  const setActive = useCallback(
    (index: number, opts: { scroll?: boolean } = {}) => {
      const next = Math.max(0, Math.min(index, count - 1));
      state.set({ active: next, engaged: opts.scroll === false ? state.value.engaged : true });
      if (opts.scroll !== false) listRef.current?.scrollIntoView({ index: next, behavior: "auto" });
      if (fetchMore && next >= count - 1) fetchMore();
    },
    [count, state, fetchMore],
  );

  const disabled = useCallback((i: number) => !!isDisabled && items[i] !== undefined && isDisabled(items[i] as T), [isDisabled, items]);
  /** The nearest enabled row from `from` going `dir`, or `from` when there is none. */
  const seek = useCallback(
    (from: number, dir: 1 | -1) => {
      for (let i = from; i >= 0 && i < count; i += dir) if (!disabled(i)) return i;
      return -1;
    },
    [count, disabled],
  );

  const pick = useCallback(
    (index: number) => {
      const item = items[index];
      if (item === undefined || disabled(index)) return;
      onPick(item, index);
    },
    [items, disabled, onPick],
  );

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      if (e.nativeEvent.isComposing || e.altKey || e.ctrlKey || e.metaKey) return;
      const go = (index: number, dir: 1 | -1) => {
        const to = seek(Math.max(0, Math.min(index, count - 1)), dir);
        if (to >= 0) setActive(to);
      };
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          go(active + 1, 1);
          break;
        case "ArrowUp":
          e.preventDefault();
          go(active - 1, -1);
          break;
        case "PageDown":
          e.preventDefault();
          go(active + PAGE_STEP, -1);
          break;
        case "PageUp":
          e.preventDefault();
          go(active - PAGE_STEP, 1);
          break;
        case "Home":
          if (count === 0) return;
          e.preventDefault();
          go(0, 1);
          break;
        case "End":
          if (count === 0) return;
          e.preventDefault();
          go(count - 1, -1);
          break;
        case "Enter":
          if (count === 0) return;
          e.preventDefault();
          pick(active);
          break;
        case "Escape":
          onEscape?.();
          break;
      }
    },
    [active, count, onEscape, pick, seek, setActive],
  );

  return {
    active,
    engaged: state.value.engaged,
    setActive,
    onKeyDown,
    optionId,
    activeId: count === 0 ? undefined : optionId(active),
    listRef,
    pick,
    count,
  };
}
