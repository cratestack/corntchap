import { useMemo, type Key, type ReactNode } from "react";
import { TableVirtuoso, type TableComponents } from "react-virtuoso";
import { cn } from "@/lib/utils";
import { useIsPhone } from "@/lib/media";
import { VirtualList } from "./virtual-list";
import { OVERSCAN_PX } from "./virtual-shared";

export interface VirtualTableColumn<T> {
  id: string;
  header: ReactNode;
  cell: (row: T) => ReactNode;
  /** On the header cell and every body cell of the column (width, alignment, text size). */
  className?: string;
}

export interface VirtualTableProps<T> {
  rows: readonly T[];
  rowKey: (row: T, index: number) => Key;
  columns: readonly VirtualTableColumn<T>[];
  /** A row on a phone, where a table does not fit: cards in a virtualized list instead. */
  card: (row: T, index: number) => ReactNode;
  /** `"window"` (default): the page scrolls and the header sticks to the top of the viewport. `"own"`: a scroller that fills its parent. */
  scroll?: "window" | "own";
  /** Fires when the end comes into view, to load the next page. */
  onEndReached?: (() => void) | undefined;
  /** After the last row: a spinner or "load more". */
  footer?: ReactNode;
  empty?: ReactNode;
  className?: string | undefined;
  "aria-label"?: string;
}

interface Context {
  className?: string | undefined;
}

const tableComponents: TableComponents<unknown, Context> = {
  Table: ({ style: _style, context: _context, ...rest }) => <table {...rest} className="w-full border-collapse text-left text-[13.5px]" />,
  TableHead: ({ context: _context, ...rest }) => <thead {...rest} className="z-10 bg-card" />,
  TableRow: ({ item: _item, context: _context, ...rest }) => <tr {...rest} className="group/row hover:bg-muted/40" />,
};

/**
 * A virtualized table: a sticky header over rows that render only near the viewport, from 768px up; on phones the
 * same rows are cards in a `VirtualList`.
 */
export function VirtualTable<T>({ rows, rowKey, columns, card, scroll = "window", onEndReached, footer, empty, className, "aria-label": label }: VirtualTableProps<T>) {
  const phone = useIsPhone();
  const context = useMemo<Context>(() => ({ className }), [className]);
  if (rows.length === 0 && empty) return <>{empty}</>;
  if (phone) {
    return (
      <div className={cn("overflow-clip rounded-[14px] border border-border bg-card", className)}>
        <VirtualList items={rows} itemKey={rowKey} renderItem={card} scroll={scroll} onEndReached={onEndReached} {...(label ? { "aria-label": label } : {})} itemClassName="border-b border-border/70 last:border-b-0" defaultItemHeight={72} />
        {footer}
      </div>
    );
  }
  return (
    <div className={cn("overflow-clip rounded-[14px] border border-border bg-card", scroll === "own" && "h-full min-h-0", className)}>
      <TableVirtuoso<T, Context>
        data={rows as T[]}
        context={context}
        components={tableComponents as TableComponents<T, Context>}
        computeItemKey={(index, row) => rowKey(row, index)}
        increaseViewportBy={{ top: OVERSCAN_PX, bottom: OVERSCAN_PX }}
        defaultItemHeight={56}
        {...(scroll === "window" ? { useWindowScroll: true } : { className: "scrollbar-thin overscroll-contain" })}
        {...(onEndReached ? { endReached: onEndReached } : {})}
        fixedHeaderContent={() => (
          <tr>
            {columns.map((c) => (
              <th key={c.id} scope="col" className={cn("label-caps border-b border-border bg-card px-4 py-2.5 text-left font-normal", c.className)}>
                {c.header}
              </th>
            ))}
          </tr>
        )}
        itemContent={(_index, row) =>
          columns.map((c) => (
            <td key={c.id} className={cn("border-b border-border/70 px-4 py-3 align-middle", c.className)}>
              {c.cell(row)}
            </td>
          ))
        }
        {...(label ? { "aria-label": label } : {})}
      />
      {footer}
    </div>
  );
}
