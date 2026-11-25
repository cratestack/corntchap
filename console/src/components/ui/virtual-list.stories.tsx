import type { Meta, StoryObj } from "@storybook/react-vite";
import { useElement } from "@/lib/use-element";
import { VirtualList } from "./virtual-list";
import { RowCard, makeRows, useGrowing, usePaged, type FixtureRow } from "./virtual.fixtures";

const rows10k = makeRows(10_000);

const meta = {
  title: "UI/VirtualList",
  component: VirtualList<FixtureRow>,
  args: { items: rows10k, itemKey: (r: FixtureRow) => r.id, renderItem: (r: FixtureRow) => <RowCard row={r} />, scroll: "own" },
  decorators: [(Story, { args }) => (args.scroll === "window" ? <Story /> : <div className="h-dvh bg-background p-4"><div className="mx-auto h-full max-w-xl overflow-hidden rounded-[14px] border border-border"><Story /></div></div>)],
} satisfies Meta<typeof VirtualList<FixtureRow>>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 10,000 rows of 1 to 4 lines each, in its own scroller. */
export const TenThousandMixedHeights: Story = {};

export const Phone: Story = {
  globals: { viewport: { value: "phone", isRotated: false } },
};

/** The page scrolls (as on phones, below a sticky header). */
export const WindowScroll: Story = {
  args: { scroll: "window" },
  decorators: [(Story) => <div className="bg-background"><header className="sticky top-0 z-10 h-14 border-b border-border bg-background px-4 py-4 font-semibold">Sticky header</header><Story /></div>],
  parameters: { layout: "fullscreen" },
};

export const Empty: Story = {
  args: { items: [], empty: <p className="py-10 text-center text-muted-foreground">Nothing here yet.</p> },
};

export const Loading: Story = {
  args: {
    items: [],
    empty: (
      <div aria-busy="true" className="space-y-2 p-4">
        {Array.from({ length: 5 }, (_, i) => <div key={i} className="h-14 animate-pulse rounded-lg bg-muted" />)}
      </div>
    ),
  },
};

/** A new row arrives every second, on top of the 400 the console keeps. */
export const LiveAppend: Story = {
  render: function Render1(args) {
    const items = useGrowing((n) => makeRows(n).reverse(), 20);
    return <VirtualList {...args} items={items} />;
  },
};

/** Paging: the footer shows while the next page loads. */
export const EndReachedWithFooter: Story = {
  render: function Render2(args) {
    const { items, loading, done, loadMore } = usePaged(makeRows, 30, 30, 200);
    return (
      <VirtualList
        {...args}
        items={items}
        onEndReached={loadMore}
        footer={<p className="py-4 text-center text-[12.5px] text-muted-foreground">{loading ? "Loading more" : done ? "That is everything" : ""}</p>}
      />
    );
  },
};

/** Scrolling inside a container that already scrolls. */
export const CustomScrollParent: Story = {
  render: function Render3(args) {
    const { element, ref } = useElement<HTMLDivElement>();
    return (
      <div ref={ref} className="h-full overflow-y-auto">
        <p className="p-4 text-sm text-muted-foreground">Content above the list</p>
        <VirtualList {...args} scroll={{ parent: element }} />
      </div>
    );
  },
};
