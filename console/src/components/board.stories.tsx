import type { Meta, StoryObj } from "@storybook/react-vite";
import { useMemo } from "react";
import { BoardView, useColumns, type BoardColumns } from "./board";
import type { Bucket } from "@/lib/format";
import { makeColumnTasks, makeTasks, usePaged } from "./ui/virtual.fixtures";

const tasks = makeTasks(10_000);

/** A board over generated tasks, bucketed like a saved view. */
function Fixture(args: Parameters<typeof BoardView>[0] & { source: typeof tasks }) {
  const columns = useColumns(args.source, false);
  return <BoardView {...args} columns={columns} />;
}

const meta = {
  title: "Console/Board",
  component: Fixture,
  args: { loading: false, source: tasks, columns: { groups: { ready: [], claimed: [], waiting: [], done: [], cancelled: [] } } satisfies BoardColumns, bare: false, selected: null, onOpen: () => undefined, onNew: () => undefined, menus: {} },
  decorators: [(Story) => <div className="min-h-dvh bg-background px-4 pt-4 md:px-5 lg:px-8"><Story /></div>],
} satisfies Meta<typeof Fixture>;
export default meta;
type Story = StoryObj<typeof meta>;

/** Desktop: columns side by side as many as fit, the page scrolls through them; 2,000 cards each. */
export const TenThousandTasks: Story = {};

/** Phone: a tab strip and one column below it; the page scrolls. */
export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };

export const Loading: Story = { args: { loading: true } };

export const Empty: Story = { args: { source: [], bare: true } };

const none = { ready: [], claimed: [], waiting: [], done: [], cancelled: [] };

/** The served column pages in 25 more tasks when its end comes into view; its header shows the server's total. */
export const ServedPaged: Story = {
  render: function Render(args) {
    const served = usePaged((n) => makeTasks(n).map((t) => ({ ...t, status: "done" })), 25, 25, 120);
    const columns = useMemo<BoardColumns>(
      () => ({ groups: { ...none, done: served.items }, totals: { done: 120 }, more: { done: { has: !served.done, loading: served.loading, load: served.loadMore } } }),
      [served.items, served.done, served.loading, served.loadMore],
    );
    return <BoardView {...args} columns={columns} />;
  },
};

const COLUMN_IDS: Bucket[] = ["ready", "claimed", "waiting", "done", "cancelled"];
const COLUMN_TOTAL = 10_000;

/**
 * Every column is its own server-paged list of 10,000 tasks (200 a page, like the server's largest): the header
 * shows the total, a column's end loads its next page, and the output below counts the pages each one asked for.
 */
function PagedColumns(args: Parameters<typeof BoardView>[0]) {
  const ready = usePaged((n) => makeColumnTasks("ready", n), 25, 200, COLUMN_TOTAL, 60);
  const claimed = usePaged((n) => makeColumnTasks("claimed", n), 25, 200, COLUMN_TOTAL, 60);
  const waiting = usePaged((n) => makeColumnTasks("waiting", n), 25, 200, COLUMN_TOTAL, 60);
  const done = usePaged((n) => makeColumnTasks("done", n), 25, 200, COLUMN_TOTAL, 60);
  const cancelled = usePaged((n) => makeColumnTasks("cancelled", n), 25, 200, COLUMN_TOTAL, 60);
  const cols = { ready, claimed, waiting, done, cancelled };
  const columns = useMemo<BoardColumns>(() => {
    const more: BoardColumns["more"] = {};
    const groups = { ready: ready.items, claimed: claimed.items, waiting: waiting.items, done: done.items, cancelled: cancelled.items };
    for (const id of COLUMN_IDS) more[id] = { has: !cols[id].done, loading: cols[id].loading, load: cols[id].loadMore };
    return { groups, totals: Object.fromEntries(COLUMN_IDS.map((id) => [id, COLUMN_TOTAL])), more };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready.items, ready.loading, claimed.items, claimed.loading, waiting.items, waiting.loading, done.items, done.loading, cancelled.items, cancelled.loading]);
  return (
    <>
      <BoardView {...args} columns={columns} />
      <output data-testid="loads" data-ready={ready.loads} data-claimed={claimed.loads} data-waiting={waiting.loads} data-done={done.loads} data-cancelled={cancelled.loads} className="sr-only">
        pages asked for: ready {ready.loads}, on the fire {claimed.loads}, waiting {waiting.loads}, served {done.loads}, tossed {cancelled.loads}
      </output>
    </>
  );
}

/** Desktop: 10,000 tasks in every column, each paged on its own as its end is scrolled to. */
export const PagedTenThousandPerColumn: Story = { render: (args) => <PagedColumns {...args} /> };

/** Phone: the same paged columns; only the column in the open tab loads more as the page scrolls. */
export const PagedPhone: Story = { render: (args) => <PagedColumns {...args} />, globals: { viewport: { value: "phone", isRotated: false } } };
