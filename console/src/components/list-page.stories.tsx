import { useMemo } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Plus } from "lucide-react";
import { staticSource } from "@/lib/listbox";
import { useUrlFilters } from "@/lib/url-filters";
import { Button } from "./ui/button";
import { FilterBar, type FacetOption } from "./filter-bar";
import { ListPage } from "./list-page";
import { errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./ui/virtual.fixtures";

const people = makePeople(10_000);
const kinds: FacetOption[] = [
  { value: "agent", label: "Agents" },
  { value: "user", label: "People" },
];

const meta: Meta = {
  title: "Console/ListPage",
  decorators: [(Story) => <div className="min-h-dvh bg-background"><Story /></div>],
};
export default meta;
type Story = StoryObj;

const Row = ({ p }: { p: FixturePerson }) => (
  <div className="flex items-baseline gap-3 px-4 py-3">
    <span className="font-mono text-[13px] font-medium">{p.name}</span>
    <span className="text-[13px] text-muted-foreground">{p.display}</span>
    <span className="ml-auto text-[12px] text-muted-foreground">{p.kind}</span>
  </div>
);
const common = { title: "Cooks", itemKey: (p: FixturePerson) => p.id, renderItem: (p: FixturePerson) => <Row p={p} />, empty: "No cooks yet.", actions: <Button size="sm"><Plus /> New cook</Button> };

/** 10,000 rows; the page itself scrolls, only the rows near the viewport are in the DOM. */
export const TenThousand: Story = {
  render: function Render() {
    const source = useMemo(() => staticSource(people), []);
    return <ListPage<FixturePerson> {...common} source={source} />;
  },
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };

const SCHEMA = { kind: { type: "string" } } as const;

/** Server-paged (25 a request) and searched, with the filters in the URL. */
export const Paged: Story = {
  render: function Render() {
    const f = useUrlFilters(SCHEMA);
    const ofKind = useMemo(() => (f.values.kind ? people.filter((p) => p.kind === f.values.kind) : people), [f.values.kind]);
    const { source, onQuery } = usePagedSource(ofKind, { match: matchPerson, initialQuery: f.q });
    return (
      <ListPage<FixturePerson>
        {...common}
        source={source}
        filters={
          <FilterBar
            q={f.q}
            onQ={(q) => {
              f.setQ(q);
              onQuery(q);
            }}
            facets={[{ id: "kind", label: "Kind", source: staticSource(kinds) }]}
            values={{ kind: f.values.kind }}
            onChange={(p) => f.set(p as never)}
            onClear={() => {
              f.clear();
              onQuery("");
            }}
          />
        }
      />
    );
  },
};

/** From 768px a table with a sticky header; phones get the same rows as cards. */
export const Table: Story = {
  render: function Render() {
    const source = useMemo(() => staticSource(people), []);
    return (
      <ListPage<FixturePerson>
        {...common}
        source={source}
        columns={[
          { id: "name", header: "Name", cell: (p) => <span className="font-mono font-medium">{p.name}</span> },
          { id: "display", header: "Display name", cell: (p) => p.display },
          { id: "kind", header: "Kind", cell: (p) => p.kind, className: "w-32" },
        ]}
      />
    );
  },
};

/** The page scrolls past a tall banner and then the list: no inner scroller. */
export const WindowScroll: Story = {
  render: function Render() {
    const source = useMemo(() => staticSource(people), []);
    return (
      <>
        <div className="h-72 bg-muted p-8 text-sm text-muted-foreground">Content above the page.</div>
        <ListPage<FixturePerson> {...common} source={source} />
      </>
    );
  },
};

export const Empty: Story = { render: () => <ListPage<FixturePerson> {...common} source={staticSource<FixturePerson>([])} /> };

export const Loading: Story = { render: () => <ListPage<FixturePerson> {...common} source={loadingSource<FixturePerson>()} /> };

export const Error: Story = { render: () => <ListPage<FixturePerson> {...common} source={errorSource<FixturePerson>()} /> };
