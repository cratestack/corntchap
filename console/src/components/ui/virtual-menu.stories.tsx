import type { Meta, StoryObj } from "@storybook/react-vite";
import { Check, Filter } from "lucide-react";
import { useStaticSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { Button } from "./button";
import { VirtualMenu } from "./virtual-menu";
import { PersonRow, PickedLine, StoryPage, errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./virtual.fixtures";

const people = makePeople(10_000);

const meta: Meta = {
  title: "UI/VirtualMenu",
  decorators: [(Story) => <StoryPage><div className="mx-auto max-w-sm"><Story /></div></StoryPage>],
};
export default meta;
type Story = StoryObj;

const common = { itemKey: (p: FixturePerson) => p.id, itemText: (p: FixturePerson) => `${p.name} ${p.display}`, renderItem: (p: FixturePerson) => <PersonRow person={p} /> };
const trigger = (
  <Button variant="outline" size="sm">
    <Filter /> Assign to
  </Button>
);

/** An action menu: a pick runs its action and closes. */
export const TenThousand: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const picked = useStoreValue("");
    return (
      <>
        <VirtualMenu<FixturePerson> label="Assign to" mode="action" trigger={trigger} source={source} onSelect={(p) => picked.set(p.id)} defaultOpen {...common} />
        <PickedLine value={picked.value} />
      </>
    );
  },
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };

/** Server-paged and searched. */
export const Paged: Story = {
  render: function Render() {
    const { source, onQuery } = usePagedSource(people, { match: matchPerson });
    const picked = useStoreValue("");
    return (
      <>
        <VirtualMenu<FixturePerson> label="Assign to" mode="action" trigger={trigger} source={source} onQuery={onQuery} onSelect={(p) => picked.set(p.id)} defaultOpen {...common} />
        <PickedLine value={picked.value} />
      </>
    );
  },
};

/** One selected (a check on it); a pick closes. */
export const Single: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const picked = useStoreValue("cook-3");
    return (
      <>
        <VirtualMenu<FixturePerson> label="Owner" mode="single" trigger={trigger} source={source} selected={(p) => p.id === picked.value} onSelect={(p) => picked.set(p.id)} defaultOpen {...common} />
        <PickedLine value={picked.value} />
      </>
    );
  },
};

/** Several selected; the menu stays open for each pick. */
export const Multiple: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const picked = useStoreValue<string[]>(["cook-1"]);
    return (
      <>
        <VirtualMenu<FixturePerson>
          label="Owners"
          mode="multi"
          trigger={trigger}
          source={source}
          selected={(p) => picked.value.includes(p.id)}
          onSelect={(p) => picked.set(picked.value.includes(p.id) ? picked.value.filter((x) => x !== p.id) : [...picked.value, p.id])}
          header={<p className="label-caps px-3 pt-2 pb-1">Owners</p>}
          defaultOpen
          {...common}
        />
        <PickedLine value={picked.value.join(",")} />
      </>
    );
  },
};

/** No search box: the list itself is focused and takes the keys. */
export const NotSearchable: Story = {
  render: function Render() {
    const source = useStaticSource(people.slice(0, 6));
    const picked = useStoreValue("");
    return (
      <>
        <VirtualMenu<FixturePerson> label="Assign to" mode="action" searchable={false} trigger={trigger} source={source} onSelect={(p) => picked.set(p.id)} defaultOpen {...common} renderItem={(p) => <span className="flex items-center gap-2"><Check className="size-3.5 opacity-0" aria-hidden="true" />{p.name}</span>} />
        <PickedLine value={picked.value} />
      </>
    );
  },
};

export const Empty: Story = {
  render: function Render() {
    const source = useStaticSource<FixturePerson>([]);
    return <VirtualMenu<FixturePerson> label="Assign to" mode="action" trigger={trigger} source={source} onSelect={() => {}} defaultOpen empty="Nobody to assign." {...common} />;
  },
};

export const Loading: Story = {
  render: () => <VirtualMenu<FixturePerson> label="Assign to" mode="action" trigger={trigger} source={loadingSource()} onSelect={() => {}} defaultOpen {...common} />,
};

export const Error: Story = {
  render: () => <VirtualMenu<FixturePerson> label="Assign to" mode="action" trigger={trigger} source={errorSource()} onSelect={() => {}} defaultOpen {...common} />,
};
