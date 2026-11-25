import type { Meta, StoryObj } from "@storybook/react-vite";
import { VirtualGroupedList, type VirtualGroup } from "./virtual-grouped-list";
import { RowCard, makeRows, useGrowing, type FixtureRow } from "./virtual.fixtures";

const groups10k: VirtualGroup[] = Array.from({ length: 100 }, (_, i) => ({ key: `g${i}`, label: `Group ${i + 1}`, count: 100 }));
const rows10k = makeRows(10_000);

const meta = {
  title: "UI/VirtualGroupedList",
  component: VirtualGroupedList<FixtureRow>,
  args: {
    groups: groups10k,
    items: rows10k,
    scroll: "own",
    itemKey: (r: FixtureRow) => r.id,
    renderItem: (r: FixtureRow) => <RowCard row={r} />,
    renderGroup: (g: VirtualGroup) => <div className="label-caps border-b border-border bg-card px-4 py-2">{g.label}</div>,
  },
  decorators: [(Story, { args }) => (args.scroll === "window" ? <Story /> : <div className="h-dvh bg-background p-4"><div className="mx-auto h-full max-w-xl overflow-hidden rounded-[14px] border border-border"><Story /></div></div>)],
} satisfies Meta<typeof VirtualGroupedList<FixtureRow>>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 100 groups of 100 rows with mixed heights; the group header sticks. */
export const TenThousandMixedHeights: Story = {};

export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };

export const WindowScroll: Story = {
  args: { scroll: "window", stickyClassName: "top-14!" },
  decorators: [(Story) => <div className="bg-background"><header className="sticky top-0 z-20 h-14 border-b border-border bg-background px-4 py-4 font-semibold">Sticky bar</header><Story /></div>],
};

export const Empty: Story = {
  args: { groups: [], items: [], empty: <p className="py-10 text-center text-muted-foreground">Nothing here yet.</p> },
};

/** New rows land in the first group every second. */
export const LiveAppend: Story = {
  render: function Render1(args) {
    const items = useGrowing((n) => makeRows(n).reverse(), 20);
    const groups: VirtualGroup[] = [{ key: "today", label: "Today", count: items.length }];
    return <VirtualGroupedList {...args} groups={groups} items={items} />;
  },
};
