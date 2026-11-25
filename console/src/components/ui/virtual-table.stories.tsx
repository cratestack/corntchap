import type { Meta, StoryObj } from "@storybook/react-vite";
import { VirtualTable, type VirtualTableColumn } from "./virtual-table";
import { makeRows, usePaged, type FixtureRow } from "./virtual.fixtures";

const rows10k = makeRows(10_000);

const columns: VirtualTableColumn<FixtureRow>[] = [
  { id: "id", header: "Id", cell: (r) => <span className="font-mono text-[12px]">{r.id}</span>, className: "w-24" },
  { id: "title", header: "Title", cell: (r) => r.title },
  { id: "lines", header: "Notes", cell: (r) => Array.from({ length: r.lines }, (_, i) => <p key={i} className="text-muted-foreground">Note {i + 1}</p>) },
];

const meta = {
  title: "UI/VirtualTable",
  component: VirtualTable<FixtureRow>,
  args: {
    rows: rows10k,
    rowKey: (r: FixtureRow) => r.id,
    columns,
    card: (r: FixtureRow) => (
      <div data-testid="row" className="px-4 py-3">
        <p className="font-mono text-[12px] font-bold">{r.title}</p>
        <p className="text-[13px] text-muted-foreground">{r.lines} notes</p>
      </div>
    ),
    "aria-label": "Rows",
  },
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 md:px-8"><Story /></div>],
} satisfies Meta<typeof VirtualTable<FixtureRow>>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 10,000 rows under a sticky header; the page scrolls. */
export const TenThousandRows: Story = {};

/** On a phone the rows are cards. */
export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };

export const Empty: Story = { args: { rows: [], empty: <p className="py-10 text-center text-muted-foreground">No rows.</p> } };

/** Paging: 30 more rows when the end comes into view, with a closing note when there are no more. */
export const EndReachedWithFooter: Story = {
  render: function Render(args) {
    const { items, loading, done, loadMore } = usePaged(makeRows, 30, 30, 200);
    return <VirtualTable {...args} rows={items} onEndReached={loadMore} footer={<p className="py-3 text-center text-[12.5px] text-muted-foreground">{loading ? "Loading more" : done ? "That is everything" : ""}</p>} />;
  },
};
