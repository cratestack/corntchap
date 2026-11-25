import { forwardRef, type KeyboardEventHandler, type Key, type ReactNode } from "react";
import type { Components, GroupProps, ItemProps, ListProps, TopItemListProps } from "react-virtuoso";
import { cn } from "@/lib/utils";

/**
 * Where a virtualized list scrolls:
 * - `"own"`: its own scroller, which fills the parent (give the parent a height);
 * - `"window"`: the page scrolls (phones: sticky safe-area headers stay put);
 * - `{ parent }`: an existing scroll container (from `useElement().element`; renders nothing until it is mounted).
 */
export type VirtualScroll = "own" | "window" | { parent: HTMLElement | null };

/** Extra pixels rendered above and below the viewport; generous so iOS momentum scrolling does not reveal blanks. */
export const OVERSCAN_PX = 900;

/** What a listbox (options with `aria-selected`, one active row) hands its Virtuoso rows through `context`. */
export interface ListboxBinding {
  /** The `role="listbox"` element's id (what a combobox's `aria-controls` names). */
  id: string;
  multiple: boolean;
  activeIndex: number;
  optionId: (index: number) => string;
  isSelected: (item: never, index: number) => boolean;
  isDisabled: (item: never, index: number) => boolean;
  pick: (index: number) => void;
  hover: (index: number) => void;
  /** When the listbox itself holds focus (no search input): the key handler and `aria-activedescendant`. */
  focusable?: { activeId: string | undefined; onKeyDown: KeyboardEventHandler<HTMLElement> } | undefined;
}

export interface VirtualContext {
  listbox?: ListboxBinding | undefined;
  /** The list's accessible name (on the `role="list"` element; Virtuoso's root is a plain div). */
  label?: string | undefined;
  itemClassName?: string | undefined;
  listClassName?: string | undefined;
  stickyClassName?: string | undefined;
  footer?: ReactNode;
  /** Before the first row (a thread's "loading earlier messages"). */
  header?: ReactNode;
}

export type VirtualKey = (index: number) => Key;

/** The Virtuoso props that choose the scroll container. */
export function scrollProps(scroll: VirtualScroll): { useWindowScroll?: boolean; customScrollParent?: HTMLElement } {
  if (scroll === "window") return { useWindowScroll: true };
  if (scroll === "own") return {};
  return scroll.parent ? { customScrollParent: scroll.parent } : {};
}

const List = forwardRef<HTMLDivElement, ListProps & { context: VirtualContext }>(function List({ context, children, ...rest }, ref) {
  // Virtuoso renders this twice in a grouped list: the rows, and the sticky group header on top (no rows: not a list)
  const top = (rest as Record<string, unknown>)["data-testid"] === "virtuoso-top-item-list";
  return (
    <div ref={ref} role={top ? "presentation" : "list"} aria-label={top ? undefined : context.label} {...rest} className={context.listClassName}>
      {children}
    </div>
  );
});

function Item({ context, item: _item, children, ...rest }: ItemProps<unknown> & { context: VirtualContext }) {
  return (
    <div role="listitem" {...rest} className={context.itemClassName}>
      {children}
    </div>
  );
}

function Group({ children, ...rest }: GroupProps) {
  return (
    <div role="presentation" {...rest}>
      {children}
    </div>
  );
}

function TopItemList({ context, children, ...rest }: TopItemListProps & { context: VirtualContext }) {
  return (
    <div role="presentation" {...rest} className={cn(context.stickyClassName)}>
      {children}
    </div>
  );
}

function Footer({ context }: { context: VirtualContext }) {
  return <>{context.footer}</>;
}

function Header({ context }: { context: VirtualContext }) {
  return <>{context.header}</>;
}

/** The Tailwind-classed pieces every virtualized list shares; classes come in through Virtuoso's `context`. */
export const virtualComponents: Components<any, VirtualContext> = { List, Item, Group, TopItemList, Footer, Header };

const ListboxList = forwardRef<HTMLDivElement, ListProps & { context: VirtualContext }>(function ListboxList({ context, children, ...rest }, ref) {
  const lb = context.listbox;
  const top = (rest as Record<string, unknown>)["data-testid"] === "virtuoso-top-item-list";
  if (top || !lb) return <div ref={ref} role="presentation" {...rest}>{children}</div>;
  return (
    <div
      ref={ref}
      role="listbox"
      id={lb.id}
      aria-label={context.label}
      aria-multiselectable={lb.multiple || undefined}
      {...(lb.focusable ? { tabIndex: 0, "aria-activedescendant": lb.focusable.activeId, onKeyDown: lb.focusable.onKeyDown } : {})}
      {...rest}
      className={cn("outline-none", context.listClassName)}
    >
      {children}
    </div>
  );
});

function ListboxItem({ context, item, children, ...rest }: ItemProps<unknown> & { context: VirtualContext }) {
  const lb = context.listbox;
  const index = rest["data-item-index"];
  if (!lb) return <div {...rest}>{children}</div>;
  const active = index === lb.activeIndex;
  const disabled = lb.isDisabled(item as never, index);
  return (
    <div
      role="option"
      id={lb.optionId(index)}
      aria-selected={lb.isSelected(item as never, index)}
      aria-disabled={disabled || undefined}
      data-active={active || undefined}
      {...rest}
      onClick={() => lb.pick(index)}
      onMouseMove={() => !active && !disabled && lb.hover(index)}
      className={cn("cursor-pointer select-none data-active:bg-accent data-active:text-accent-foreground aria-disabled:cursor-default aria-disabled:opacity-50", context.itemClassName)}
    >
      {children}
    </div>
  );
}

/** The pieces of a virtualized listbox: `role="listbox"` List, `role="option"` Item with `aria-selected` and an id per row. */
export const listboxComponents: Components<any, VirtualContext> = { List: ListboxList, Item: ListboxItem, Group, TopItemList, Footer, Header };
