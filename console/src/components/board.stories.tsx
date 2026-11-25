import type { Meta, StoryObj } from "@storybook/react-vite";
import { useMemo } from "react";
import { BoardView, useColumns, type BoardColumns } from "./board";
import { makeTasks, usePaged } from "./ui/virtual.fixtures";

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

/** Desktop: five columns, each with its own scroller; 2,000 cards each. */
export const TenThousandTasks: Story = {};

/** Phone: snap columns with a tab strip; the page scrolls. */
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
