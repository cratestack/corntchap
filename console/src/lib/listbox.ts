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
  /**
   * The rows are not the current query's yet (the search text has not rested, or its first page is loading): they
   * may be shown, but nothing is picked from them; a pick waits for the answer.
   */
  pending?: boolean | undefined;
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
  /**
   * No row is active until the user moves to one with the keys (a text field whose Enter must keep its own meaning):
   * nothing is highlighted, `aria-activedescendant` is unset and Enter picks nothing; the first ArrowDown makes the
   * first row active (ArrowUp, the last).
   */
  requireEngage?: boolean | undefined;
  /** The rows are the previous query's (see `ListSource.pending`), for all rows or, given a row, for that one (undefined: no rows). */
  pending?: boolean | ((item: T | undefined) => boolean) | undefined;
  /** A row's identity: the active row stays on the same item when rows are added before it. */
  itemId?: ((item: T) => string) | undefined;
}

export interface ListboxController {
  /** Index of the active row, clamped to the rows there are (what Enter picks). */
  active: number;
  /** The user moved the active row with the keys (not just the default first row). */
  engaged: boolean;
  /** The row to draw as active: `active`, or -1 when `requireEngage` and not engaged yet. */
  highlight: number;
  /** Make row `index` the active one (what the pointer does: it passes `{ scroll: false }`). */
  setActive: (index: number, opts?: { scroll?: boolean }) => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
  optionId: (index: number) => string;
  /** The id to give `aria-activedescendant`; undefined when there is nothing to name: no rows, no highlight, or the row is not rendered (scrolled away). */
  activeId: string | undefined;
  /** Hand to the list's `ref`: the active row is scrolled into view through it. */
  listRef: RefObject<ScrollsIntoView | null>;
  /** Hand to the list's `rangeChanged`: the rows rendered changed (so `activeId` is re-checked). */
  onRendered: () => void;
  /** Pick row `index` now; nothing while the rows are `pending`. */
  pick: (index: number) => void;
  /** Pick the active row (or `index`), or, while the rows are pending, as soon as the answer is in. */
  commit: (index?: number) => void;
  /** Back to the first row, not engaged (after a pick that changes what is listed). */
  reset: () => void;
  count: number;
}

interface ListboxState {
  active: number;
  engaged: boolean;
  /** Enter was pressed while the rows were pending: pick when they are not. */
  queued: boolean;
  /** The last move was End: keep following the end as more rows load. */
  end: boolean;
  /** The active item's id (with `itemId`): the active row follows it when rows come before it. */
  key: string | undefined;
}

const INITIAL: ListboxState = { active: 0, engaged: false, queued: false, end: false, key: undefined };

/**
 * The keyboard and active-row logic of a listbox, apart from how it looks: ArrowUp / ArrowDown, Home / End, PageUp /
 * PageDown move the active row (skipping disabled ones; it is scrolled into view, even rows not rendered yet),
 * Enter picks it, Escape calls `onEscape`. The element that has focus (the search input, or the listbox itself) gets
 * `onKeyDown` and `aria-activedescendant={activeId}`; focus stays there while the options are virtualized.
 */
export function useListbox<T>({ items, onPick, isDisabled, idPrefix, resetOn, onEscape, fetchMore, requireEngage = false, pending, itemId }: ListboxOptions<T>): ListboxController {
  const state = useStoreValue<ListboxState>(INITIAL);
  const rendered = useStoreValue(true);
  const listRef = useRef<ScrollsIntoView | null>(null);
  const count = items.length;
  const key = state.value.key;
  const at = useMemo(() => (itemId && key !== undefined ? items.findIndex((it) => itemId(it) === key) : -1), [items, key, itemId]);
  const active = count === 0 ? 0 : at >= 0 ? at : Math.min(state.value.active, count - 1);
  const { engaged } = state.value;

  useEffect(() => {
    state.set(INITIAL);
    // only a change of `resetOn` resets
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetOn]);

  const optionId = useCallback((index: number) => `${idPrefix}-opt-${index}`, [idPrefix]);

  const move = useCallback(
    (index: number, opts: { scroll?: boolean; end?: boolean } = {}) => {
      const next = Math.max(0, Math.min(index, count - 1));
      const item = items[next];
      state.set({ active: next, engaged: true, queued: false, end: opts.end === true, key: itemId && item !== undefined ? itemId(item) : undefined });
      if (opts.scroll !== false) listRef.current?.scrollIntoView({ index: next, behavior: "auto" });
      if (fetchMore && next >= count - 1) fetchMore();
    },
    [count, items, itemId, state, fetchMore],
  );
  const setActive = useCallback((index: number, opts: { scroll?: boolean } = {}) => move(index, opts), [move]);

  const disabled = useCallback((i: number) => !!isDisabled && items[i] !== undefined && isDisabled(items[i] as T), [isDisabled, items]);
  /** The nearest enabled row from `from` going `dir`, or -1 when there is none. */
  const seek = useCallback(
    (from: number, dir: 1 | -1) => {
      for (let i = from; i >= 0 && i < count; i += dir) if (!disabled(i)) return i;
      return -1;
    },
    [count, disabled],
  );

  const pendingAt = useCallback((i: number) => (typeof pending === "function" ? pending(items[i]) : !!pending), [pending, items]);

  const pick = useCallback(
    (index: number) => {
      const item = items[index];
      if (item === undefined || disabled(index) || pendingAt(index)) return;
      onPick(item, index);
    },
    [items, disabled, pendingAt, onPick],
  );
  const commit = useCallback(
    (index: number = active) => {
      if (pendingAt(index)) state.set({ ...state.value, queued: true });
      else pick(index);
    },
    [active, pendingAt, pick, state],
  );
  const reset = useCallback(() => state.set(INITIAL), [state]);

  // an Enter that came while the rows were the previous query's picks once the answer is in
  useEffect(() => {
    if (!state.value.queued || pendingAt(active)) return;
    state.set({ ...state.value, queued: false });
    pick(active);
  });

  const toEnd = useCallback(() => {
    const to = seek(count - 1, -1);
    if (to >= 0) move(to, { end: true });
  }, [seek, count, move]);
  // End before every page is loaded: as more rows come in, the end moves, and the active row follows it
  const following = state.value.end;
  useEffect(() => {
    if (!following || count === 0) return;
    toEnd();
    // the list takes the new rows a frame later than this effect: scroll to the end once more then
    const frame = requestAnimationFrame(() => {
      const to = seek(count - 1, -1);
      if (to >= 0) listRef.current?.scrollIntoView({ index: to, behavior: "auto" });
    });
    return () => cancelAnimationFrame(frame);
    // only more (or fewer) rows move the end
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [count]);

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      if (e.nativeEvent.isComposing || e.altKey || e.ctrlKey || e.metaKey) return;
      const go = (index: number, dir: 1 | -1) => {
        const to = seek(Math.max(0, Math.min(index, count - 1)), dir);
        if (to >= 0) move(to);
      };
      const fresh = requireEngage && !engaged;
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          go(fresh ? 0 : active + 1, 1);
          break;
        case "ArrowUp":
          e.preventDefault();
          if (fresh) go(count - 1, -1);
          else go(active - 1, -1);
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
          toEnd();
          break;
        case "Enter":
          if (fresh || (count === 0 && !pendingAt(0))) return;
          e.preventDefault();
          commit();
          break;
        case "Escape":
          onEscape?.();
          break;
      }
    },
    [active, count, engaged, requireEngage, onEscape, commit, pendingAt, seek, move, toEnd],
  );

  // After a wheel or touch scroll the active row may be out of the rendered range: `aria-activedescendant` must not name it.
  const activeNow = useRef(active);
  const markRendered = rendered.set;
  const onRendered = useCallback(() => {
    const check = () => markRendered(document.getElementById(optionId(activeNow.current)) !== null);
    requestAnimationFrame(check);
    setTimeout(check, 150);
  }, [markRendered, optionId]);
  useEffect(() => {
    activeNow.current = active;
    onRendered();
  }, [active, count, onRendered]);

  const highlight = count === 0 || (requireEngage && !engaged) ? -1 : active;
  return {
    active,
    engaged,
    highlight,
    setActive,
    onKeyDown,
    optionId,
    activeId: highlight < 0 || !rendered.value ? undefined : optionId(active),
    listRef,
    onRendered,
    pick,
    commit,
    reset,
    count,
  };
}
