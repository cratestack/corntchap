import type { Meta, StoryObj } from "@storybook/react-vite";
import { VirtualColumn } from "./virtual-column";
import { RowCard, makeRows, usePaged, type FixtureRow } from "./virtual.fixtures";

const Header = ({ count }: { count: number }) => (
  <h2 className="flex items-center gap-2 px-4 pt-3.5 pb-2.5 text-[15px]">
    <span className="size-2.5 rounded-full bg-corn" aria-hidden="true" />
    Ready to scoop
    <span className="font-mono text-[11.5px] font-normal text-muted-foreground">{count}</span>
  </h2>
);

const rows10k = makeRows(10_000);

const meta = {
  title: "UI/VirtualColumn",
  component: VirtualColumn<FixtureRow>,
  args: {
    header: <Header count={rows10k.length} />,
    items: rows10k,
    itemKey: (r: FixtureRow) => r.id,
    renderItem: (r: FixtureRow) => <RowCard row={r} />,
    className: "w-[19rem]",
    headerClassName: "sticky top-0 z-10 rounded-t-[14px] bg-column",
    bodyClassName: "px-2.5 pb-2.5",
  },
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4"><Story /></div>],
} satisfies Meta<typeof VirtualColumn<FixtureRow>>;
export default meta;
type Story = StoryObj<typeof meta>;

/** A column is as tall as its cards and the page scrolls through it; its header sticks to the top. */
export const TenThousandMixedHeights: Story = {};

export const Empty: Story = {
  args: { items: [], header: <Header count={0} />, empty: <p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">Nothing here.</p> },
};

/** Phone: a column as wide as the screen; the page scrolls through it. */
export const Phone: Story = {
  args: { className: "p-2", header: null, bodyClassName: undefined, headerClassName: undefined },
  globals: { viewport: { value: "phone", isRotated: false } },
};

/** Server paging (Served, Tossed): the footer shows while the next page loads. */
export const EndReachedWithFooter: Story = {
  render: function Render1(args) {
    const { items, loading, done, loadMore } = usePaged(makeRows, 20, 20, 200);
    return (
      <VirtualColumn
        {...args}
        header={<Header count={items.length} />}
        items={items}
        onEndReached={loadMore}
        footer={<p className="py-3 text-center text-[12.5px] text-muted-foreground">{loading ? "Loading more" : done ? "That is everything" : ""}</p>}
      />
    );
  },
};
