import type { Meta, StoryObj } from "@storybook/react-vite";
import { BoardView } from "./board";
import { makeTasks } from "./ui/virtual.fixtures";

const meta = {
  title: "Console/Board",
  component: BoardView,
  args: { loading: false, tasks: makeTasks(10_000), total: 10_000, selected: null, onOpen: () => undefined, onNew: () => undefined, menus: {} },
  decorators: [(Story) => <div className="min-h-dvh bg-background px-4 pt-4 md:px-5 lg:px-8"><Story /></div>],
} satisfies Meta<typeof BoardView>;
export default meta;
type Story = StoryObj<typeof meta>;

/** Desktop: five columns, each with its own scroller; 2,000 cards each. */
export const TenThousandTasks: Story = {};

/** Phone: snap columns with a tab strip; the page scrolls. */
export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };

export const Loading: Story = { args: { loading: true } };

export const Empty: Story = { args: { tasks: [], total: 0 } };
