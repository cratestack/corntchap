import { forwardRef, type Key, type ReactNode } from "react";
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

export interface VirtualContext {
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
