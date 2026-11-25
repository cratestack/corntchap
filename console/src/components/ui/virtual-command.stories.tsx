import { useMemo } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Flame, Plus, Soup } from "lucide-react";
import { matchText, staticSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { VirtualCommand, type CommandItem, type CommandSection } from "./virtual-command";
import { PickedLine, errorSource, loadingSource, makePeople, makeLabels, usePagedSource } from "./virtual.fixtures";

const people = makePeople(10_000);
const labels = makeLabels(10_000);

const meta: Meta = { title: "UI/VirtualCommand" };
export default meta;
type Story = StoryObj;

function useStory(set: (id: string) => void) {
  return useMemo(() => {
    const actions: CommandItem[] = [
      { id: "new-task", label: "New task", keywords: ["add", "create"], icon: <Plus />, hint: "N", onSelect: () => set("new-task") },
      { id: "new-menu", label: "New menu", keywords: ["milestone"], icon: <Soup />, onSelect: () => set("new-menu") },
      { id: "fires", label: "Go to fires", icon: <Flame />, onSelect: () => set("fires") },
    ];
    const cooks: CommandItem[] = people.map((p) => ({ id: p.id, label: p.name, keywords: [p.display], hint: p.kind, onSelect: () => set(p.id) }));
    const tags: CommandItem[] = labels.map((l) => ({ id: l, label: l, onSelect: () => set(l) }));
    return { actions, cooks, tags };
  }, [set]);
}

function Palette({ sections }: { sections: (items: ReturnType<typeof useStory>) => CommandSection[] }) {
  const picked = useStoreValue("");
  const query = useStoreValue("");
  const items = useStory(picked.set);
  return (
    <div className="min-h-dvh bg-background p-8">
      <PickedLine value={picked.value} />
      <VirtualCommand open onOpenChange={() => {}} title="Search or jump" query={query.value} onQuery={query.set} sections={sections(items)} />
    </div>
  );
}

/** Sections of 3 and 10,000 rows, narrowed here by the query: one active row across them, sticky headings. */
export const TenThousand: Story = {
  render: () => <Palette sections={(i) => [
    { id: "do", heading: "Do", source: staticSource(i.actions), filter: "client" },
    { id: "cooks", heading: "Cooks", source: staticSource(i.cooks), filter: "client" },
  ]} />,
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };

/** Three sections, the last two of 10,000 rows each. */
export const Grouped: Story = {
  render: () => <Palette sections={(i) => [
    { id: "do", heading: "Do", source: staticSource(i.actions), filter: "client" },
    { id: "cooks", heading: "Cooks", source: staticSource(i.cooks), filter: "client" },
    { id: "labels", heading: "Labels", source: staticSource(i.tags), filter: "client" },
  ]} />,
};

/** A section the server pages and searches (250 ms a request); the query goes to its source. */
export const Paged: Story = {
  render: function Render() {
    const picked = useStoreValue("");
    const query = useStoreValue("");
    const i = useStory(picked.set);
    const { source, onQuery } = usePagedSource(i.cooks, { match: (it, q) => matchText(q, it.label, ...(it.keywords ?? [])) });
    return (
      <div className="min-h-dvh bg-background p-8">
        <PickedLine value={picked.value} />
        <VirtualCommand
          open
          onOpenChange={() => {}}
          title="Search or jump"
          query={query.value}
          onQuery={(q) => {
            query.set(q);
            onQuery(q);
          }}
          sections={[
            { id: "do", heading: "Do", source: staticSource(i.actions), filter: "client" },
            { id: "cooks", heading: "Cooks", source, filter: "server" },
          ]}
        />
      </div>
    );
  },
};

export const Empty: Story = { render: () => <Palette sections={() => [{ id: "do", heading: "Do", source: staticSource<CommandItem>([]), filter: "client" }]} /> };

export const Loading: Story = { render: () => <Palette sections={() => [{ id: "cooks", heading: "Cooks", source: loadingSource<CommandItem>(), filter: "server" }]} /> };

export const Error: Story = { render: () => <Palette sections={() => [{ id: "cooks", heading: "Cooks", source: errorSource<CommandItem>(), filter: "server" }]} /> };
