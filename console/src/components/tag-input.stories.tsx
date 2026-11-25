import type { Meta, StoryObj } from "@storybook/react-vite";
import { useStoreValue } from "@/lib/use-store-value";
import { TagInput } from "./tag-input";
import { PickedLine, errorSource, loadingSource, makeLabels, matchString, usePagedSource } from "./ui/virtual.fixtures";

const labels = makeLabels(10_000);

const meta: Meta = {
  title: "Console/TagInput",
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 md:p-8"><div className="mx-auto max-w-md"><Story /></div></div>],
};
export default meta;
type Story = StoryObj;

const lower = (s: string) => s.toLowerCase();

/** 10,000 suggestions, narrowed as you type; arrows then Enter or Tab take one, Enter or comma adds the typed text. */
export const TenThousandSuggestions: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>(["backend"]);
    return (
      <>
        <TagInput label="Labels" value={tags.value} onChange={tags.set} suggestions={labels} normalize={lower} placeholder="add a label" autoFocus />
        <PickedLine value={tags.value.join(",")} />
      </>
    );
  },
};

export const Phone: Story = { ...TenThousandSuggestions, globals: { viewport: { value: "phone", isRotated: false } } };

/**
 * How it sits in a form (new task, task sheet): nothing is highlighted until an arrow key is pressed (the first
 * ArrowDown highlights the first option), Enter on an empty field submits the form, Enter after an arrow takes the
 * option, and Tab moves on to the next field without adding anything more.
 */
export const InsideForm: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    const submits = useStoreValue(0);
    return (
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submits.set(submits.value + 1);
        }}
      >
        <TagInput label="Labels" value={tags.value} onChange={tags.set} suggestions={labels} normalize={lower} placeholder="add a label" autoFocus />
        <input aria-label="Next field" placeholder="the next field" className="h-9 w-full rounded-md border border-input bg-card px-3 text-sm" />
        <button type="submit" className="h-9 rounded-md border border-input px-3 text-sm">
          Save
        </button>
        <PickedLine value={tags.value.join(",")} />
        <p className="text-[12.5px] text-muted-foreground">
          Submitted: <span data-testid="submits" className="font-mono text-foreground">{submits.value}</span>
        </p>
      </form>
    );
  },
};

/** The server pages and searches the options (250 ms a request): an Enter typed before the answer waits for it. */
export const Paged: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    const { source, onQuery } = usePagedSource(labels, { match: matchString });
    return (
      <>
        <TagInput label="Labels" value={tags.value} onChange={tags.set} source={source} onQuery={onQuery} normalize={lower} placeholder="add a label" autoFocus />
        <PickedLine value={tags.value.join(",")} />
      </>
    );
  },
};

/** Only the options can be added: Enter takes the first match. */
export const OnlyKnownTags: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    return (
      <>
        <TagInput label="Labels" value={tags.value} onChange={tags.set} suggestions={labels.slice(0, 40)} allowNew={false} placeholder="pick a label" autoFocus />
        <PickedLine value={tags.value.join(",")} />
      </>
    );
  },
};

/** No options at all (how it has always worked): free text, Enter or comma adds it. */
export const FreeText: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>(["urgent"]);
    return <TagInput label="Labels" value={tags.value} onChange={tags.set} placeholder="backend, urgent" normalize={lower} />;
  },
};

export const Empty: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    return <TagInput label="Labels" value={tags.value} onChange={tags.set} suggestions={[]} placeholder="add a label" autoFocus />;
  },
};

export const Loading: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    return <TagInput label="Labels" value={tags.value} onChange={tags.set} source={loadingSource()} autoFocus />;
  },
};

export const Error: Story = {
  render: function Render() {
    const tags = useStoreValue<string[]>([]);
    return <TagInput label="Labels" value={tags.value} onChange={tags.set} source={errorSource()} autoFocus />;
  },
};
