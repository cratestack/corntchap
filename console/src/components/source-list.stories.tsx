import type { Meta, StoryObj } from "@storybook/react-vite";
import { Group } from "./bits";
import { LoadMore, SourceList } from "./source-list";
import { errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./ui/virtual.fixtures";
import { staticSource } from "@/lib/listbox";

const people = makePeople(10_000);

const meta: Meta = {
  title: "Console/SourceList",
  decorators: [
    (Story) => (
      <div className="min-h-dvh bg-background p-4 md:p-8">
        <div className="mx-auto max-w-xl">
          <Story />
        </div>
      </div>
    ),
  ],
};
export default meta;
type Story = StoryObj;

const Row = ({ p }: { p: FixturePerson }) => (
  <div className="flex items-baseline gap-3 px-4 py-3">
    <span className="font-mono text-[13px] font-medium">{p.name}</span>
    <span className="text-[13px] text-muted-foreground">{p.display}</span>
  </div>
);
const common = { itemKey: (p: FixturePerson) => p.id, renderItem: (p: FixturePerson) => <Row p={p} />, label: "Chefs", empty: <>Nobody is in this team yet.</> };

/** A section of a details page: 10,000 rows, 25 a request, the page itself scrolls and the next page loads at the end. */
export const Paged: Story = {
  render: function Render() {
    const { source } = usePagedSource(people, { match: matchPerson });
    return (
      <Group title="Chefs">
        <SourceList<FixturePerson> {...common} source={source} />
      </Group>
    );
  },
};

export const Phone: Story = { ...Paged, globals: { viewport: { value: "phone", isRotated: false } } };

export const Loading: Story = {
  render: () => (
    <Group title="Chefs">
      <SourceList<FixturePerson> {...common} source={loadingSource<FixturePerson>()} />
    </Group>
  ),
};

export const Empty: Story = {
  render: () => (
    <Group title="Chefs">
      <SourceList<FixturePerson> {...common} source={staticSource<FixturePerson>([])} />
    </Group>
  ),
};

export const Failed: Story = {
  render: () => (
    <Group title="Chefs">
      <SourceList<FixturePerson> {...common} source={errorSource<FixturePerson>()} />
    </Group>
  ),
};

/** For a plain list drawn from an array: the next page is read when this comes into view. */
export const LoadMoreSentinel: Story = {
  render: function Render() {
    const { source } = usePagedSource(people, { match: matchPerson, pageSize: 10 });
    return (
      <Group title="Sessions">
        <ul className="divide-y divide-border">
          {source.items.map((p) => (
            <li key={p.id}>
              <Row p={p} />
            </li>
          ))}
        </ul>
        <div className="px-4">
          <LoadMore source={source} />
        </div>
      </Group>
    );
  },
};
