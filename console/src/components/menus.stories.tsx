import type { Meta, StoryObj } from "@storybook/react-vite";
import type { MenuView } from "@corntchap/api";
import { MenuCard, MenuCards } from "./menus";
import { loadingSource, usePagedSource } from "./ui/virtual.fixtures";

const NOW = Date.now();

function makeMenus(count: number): MenuView[] {
  return Array.from({ length: count }, (_, i) => {
    const total = 4 + (i % 9);
    const done = i % (total + 1);
    return {
      key: `menu-${i + 1}`,
      project: "PAY",
      name: `Menu ${i + 1}`,
      description: i % 3 === 0 ? "What this menu delivers, in a sentence or two." : "",
      due_on: i % 4 === 0 ? new Date(NOW + (i % 20) * 86_400_000).toISOString().slice(0, 10) : null,
      status: "open",
      position: i,
      served_at: null,
      created_at: new Date(NOW - i * 3_600_000).toISOString(),
      updated_at: new Date(NOW - i * 60_000).toISOString(),
      total,
      done,
      open: total - done,
      claimed: 0,
      blocked: 0,
      progress: done / total,
    } as MenuView;
  });
}

const menus = makeMenus(10_000);

const meta = {
  title: "Console/MenuCards",
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 md:px-8"><div className="mx-auto max-w-3xl"><Story /></div></div>],
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

const match = (m: MenuView, q: string) => `${m.key} ${m.name}`.toLowerCase().includes(q.trim().toLowerCase());

/** 10,000 menus read 25 at a time (250 ms a page): the page scrolls, the next page loads as the end comes into view. */
export const TenThousand: Story = {
  render: function Render() {
    const { source } = usePagedSource(menus, { match });
    return <MenuCards label="Menus cooking" source={source} render={(m) => <MenuCard potKey="PAY" menu={m} onEdit={() => {}} onServe={() => {}} />} />;
  },
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };

/** The first page has not answered yet. */
export const Loading: Story = {
  render: () => <MenuCards label="Menus cooking" source={loadingSource<MenuView>()} render={(m) => <MenuCard potKey="PAY" menu={m} onEdit={() => {}} />} />,
};
