import type { Meta, StoryObj } from "@storybook/react-vite";
import { useStaticSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { Check } from "lucide-react";
import { VirtualListbox } from "./virtual-listbox";
import { PersonRow, PickedLine, StoryPage, errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./virtual.fixtures";

const people = makePeople(10_000);
const few = makePeople(3);

const meta: Meta = {
  title: "UI/VirtualListbox",
  decorators: [(Story) => <StoryPage><div className="mx-auto max-w-sm rounded-[14px] border border-border bg-popover p-1"><Story /></div></StoryPage>],
};
export default meta;
type Story = StoryObj;

function Single({ all }: { all: FixturePerson[] }) {
  const source = useStaticSource(all);
  const picked = useStoreValue("");
  return (
    <>
      <VirtualListbox<FixturePerson>
        label="Cooks"
        source={source}
        itemKey={(p) => p.id}
        selected={(p) => p.id === picked.value}
        onPick={(p) => picked.set(p.id)}
        renderItem={(p, s) => (
          <>
            <PersonRow person={p} />
            {s.selected && <Check className="ml-auto size-4 text-primary" aria-hidden="true" />}
          </>
        )}
      />
      <PickedLine value={picked.value} />
    </>
  );
}

/** 10,000 options: only the rows near the viewport are in the DOM; the list is one tab stop and takes the arrow keys. */
export const TenThousand: Story = { render: () => <Single all={people} /> };

export const Phone: Story = { render: () => <Single all={people} />, globals: { viewport: { value: "phone", isRotated: false } } };

/** The server pages it (25 at a time, 250 ms each): the footer shows while the next page loads. */
export const Paged: Story = {
  render: function Render() {
    const { source } = usePagedSource(people, { match: matchPerson });
    return <VirtualListbox<FixturePerson> label="Cooks" source={source} itemKey={(p) => p.id} renderItem={(p) => <PersonRow person={p} />} />;
  },
};

/** Several selected at once: `aria-multiselectable`, each pick toggles. */
export const Multiple: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const picked = useStoreValue<string[]>([]);
    return (
      <>
        <VirtualListbox<FixturePerson>
          label="Cooks"
          multiple
          source={source}
          itemKey={(p) => p.id}
          selected={(p) => picked.value.includes(p.id)}
          onPick={(p) => picked.set(picked.value.includes(p.id) ? picked.value.filter((x) => x !== p.id) : [...picked.value, p.id])}
          renderItem={(p, s) => (
            <>
              <PersonRow person={p} />
              {s.selected && <Check className="ml-auto size-4 text-primary" aria-hidden="true" />}
            </>
          )}
        />
        <PickedLine value={picked.value.join(",")} />
      </>
    );
  },
};

export const FewOptions: Story = { render: () => <Single all={few} /> };

export const Empty: Story = {
  render: function Render() {
    const source = useStaticSource<FixturePerson>([]);
    return <VirtualListbox<FixturePerson> label="Cooks" source={source} itemKey={(p) => p.id} renderItem={(p) => <PersonRow person={p} />} empty="No cooks yet." />;
  },
};

export const Loading: Story = {
  render: () => <VirtualListbox<FixturePerson> label="Cooks" source={loadingSource()} itemKey={(p) => p.id} renderItem={(p) => <PersonRow person={p} />} />,
};

export const Error: Story = {
  render: () => <VirtualListbox<FixturePerson> label="Cooks" source={errorSource()} itemKey={(p) => p.id} renderItem={(p) => <PersonRow person={p} />} />,
};
