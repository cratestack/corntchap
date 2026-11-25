import type { Meta, StoryObj } from "@storybook/react-vite";
import { useStaticSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { VirtualCombobox } from "./virtual-combobox";
import { PersonRow, PickedLine, StoryPage, errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./virtual.fixtures";

const people = makePeople(10_000);

const meta: Meta = {
  title: "UI/VirtualCombobox",
  decorators: [(Story) => <StoryPage><div className="mx-auto max-w-sm"><Story /></div></StoryPage>],
};
export default meta;
type Story = StoryObj;

const common = { itemKey: (p: FixturePerson) => p.id, itemText: (p: FixturePerson) => p.name, renderItem: (p: FixturePerson) => <PersonRow person={p} /> };

function Basic({ nullOption }: { nullOption?: string }) {
  const source = useStaticSource(people);
  const value = useStoreValue<string | null>(null);
  return (
    <>
      <VirtualCombobox<FixturePerson> label="Cook" placeholder="Pick a cook" source={source} value={value.value} onChange={value.set} defaultOpen {...(nullOption ? { nullOption } : {})} {...common} />
      <PickedLine value={value.value ?? ""} />
    </>
  );
}

/** 10,000 options, searched here: open by default; type to narrow, arrows and Enter to pick. */
export const TenThousand: Story = { render: () => <Basic /> };

export const Phone: Story = { render: () => <Basic />, globals: { viewport: { value: "phone", isRotated: false } } };

/** The first option clears the choice. */
export const WithNullOption: Story = { render: () => <Basic nullOption="Anyone" /> };

/** The server pages and searches (250 ms a request): the search text goes to `onQuery`. */
export const Paged: Story = {
  render: function Render() {
    const { source, onQuery } = usePagedSource(people, { match: matchPerson });
    const value = useStoreValue<string | null>(null);
    return (
      <>
        <VirtualCombobox<FixturePerson> label="Cook" source={source} onQuery={onQuery} value={value.value} onChange={value.set} defaultOpen {...common} />
        <PickedLine value={value.value ?? ""} />
      </>
    );
  },
};

/** Several at once; the popup stays open for each pick. */
export const Multiple: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const value = useStoreValue<string[]>(["cook-2", "cook-3"]);
    return (
      <>
        <VirtualCombobox<FixturePerson> multiple label="Cooks" placeholder="Pick cooks" source={source} value={value.value} onChange={value.set} defaultOpen {...common} />
        <PickedLine value={value.value.join(",")} />
      </>
    );
  },
};

/** The small trigger of a filter chip. */
export const Pill: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const value = useStoreValue<string | null>("cook-7");
    return <VirtualCombobox<FixturePerson> trigger="pill" label="Owner" source={source} value={value.value} onChange={value.set} nullOption="Anyone" {...common} />;
  },
};

/** Closed: a form field showing the chosen cook, or its placeholder. */
export const Closed: Story = {
  render: function Render() {
    const source = useStaticSource(people);
    const value = useStoreValue<string | null>("cook-42");
    return <VirtualCombobox<FixturePerson> label="Cook" source={source} value={value.value} onChange={value.set} {...common} />;
  },
};

export const Empty: Story = {
  render: function Render() {
    const source = useStaticSource<FixturePerson>([]);
    return <VirtualCombobox<FixturePerson> label="Cook" source={source} value={null} onChange={() => {}} defaultOpen {...common} />;
  },
};

export const Loading: Story = {
  render: () => <VirtualCombobox<FixturePerson> label="Cook" source={loadingSource()} value={null} onChange={() => {}} defaultOpen {...common} />,
};

export const Error: Story = {
  render: () => <VirtualCombobox<FixturePerson> label="Cook" source={errorSource()} value={null} onChange={() => {}} defaultOpen {...common} />,
};
