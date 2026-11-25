import type { Meta, StoryObj } from "@storybook/react-vite";
import { staticSource, useStaticSource, type ListSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { MentionPopup, insertMention, parseMention } from "./mention-popup";
import { PersonRow, PickedLine, errorSource, loadingSource, makePeople, matchPerson, usePagedSource, type FixturePerson } from "./ui/virtual.fixtures";

const chefs = makePeople(10_000);

const meta: Meta = {
  title: "Console/MentionPopup",
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 pt-80 md:p-8 md:pt-80"><div className="mx-auto max-w-xl"><Story /></div></div>],
};
export default meta;
type Story = StoryObj;

/** A message box whose `@` mentions open the popup; `sourceFor` makes the options for what follows the `@`. */
function Box({ sourceFor, onQuery }: { sourceFor: (q: string) => ListSource<FixturePerson>; onQuery?: (q: string) => void }) {
  const text = useStoreValue("Can you look at this, @");
  const caret = useStoreValue(23);
  const picked = useStoreValue("");
  const mention = parseMention(text.value, caret.value);
  const source = sourceFor(mention?.q ?? "");
  return (
    <>
      <MentionPopup<FixturePerson>
        open={mention !== null}
        query={mention?.q ?? ""}
        source={source}
        itemKey={(p) => p.id}
        renderItem={(p) => <PersonRow person={p} />}
        onPick={(p) => {
          if (!mention) return;
          const next = insertMention(text.value, caret.value, mention, p.name);
          text.set(next.text);
          caret.set(next.caret);
          picked.set(p.name);
        }}
        onClose={() => caret.set(0)}
        anchor={(bind) => (
          <textarea
            {...bind}
            rows={3}
            aria-label="Write to the pass"
            value={text.value}
            onChange={(e) => {
              text.set(e.target.value);
              caret.set(e.target.selectionStart ?? e.target.value.length);
              onQuery?.(parseMention(e.target.value, e.target.selectionStart ?? e.target.value.length)?.q ?? "");
            }}
            onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
            onSelect={(e) => caret.set(e.currentTarget.selectionStart ?? 0)}
            className="w-full resize-none rounded-[22px] border border-border bg-card px-4 py-3 text-base outline-none focus-visible:ring-3 focus-visible:ring-ring/15"
          />
        )}
      />
      <PickedLine value={picked.value} />
    </>
  );
}

/** 10,000 chefs, narrowed here by what follows the `@`: the popup opens above the box. */
export const TenThousandChefs: Story = {
  render: function Render() {
    const all = useStaticSource(chefs);
    return <Box sourceFor={(q) => (q ? staticSource(chefs.filter((c) => matchPerson(c, q))) : all)} />;
  },
};

export const Phone: Story = { ...TenThousandChefs, globals: { viewport: { value: "phone", isRotated: false } } };

/** The server pages and searches. */
export const Paged: Story = {
  render: function Render() {
    const { source, onQuery } = usePagedSource(chefs, { match: matchPerson });
    return <Box sourceFor={() => source} onQuery={onQuery} />;
  },
};

export const Empty: Story = {
  render: function Render() {
    const none = useStaticSource<FixturePerson>([]);
    return <Box sourceFor={() => none} />;
  },
};

export const Loading: Story = { render: () => <Box sourceFor={() => loadingSource<FixturePerson>()} /> };

export const Error: Story = { render: () => <Box sourceFor={() => errorSource<FixturePerson>()} /> };
