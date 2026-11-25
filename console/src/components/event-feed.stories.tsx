import type { Meta, StoryObj } from "@storybook/react-vite";
import { EventFeed } from "./event-feed";
import { makeEvents, useGrowing, usePaged } from "./ui/virtual.fixtures";

const meta = {
  title: "Console/EventFeed",
  component: EventFeed,
  args: { events: makeEvents(10_000) },
  decorators: [(Story) => <div className="min-h-dvh bg-background p-4 md:px-8"><div className="mx-auto max-w-3xl rounded-[14px] border border-border bg-card px-4 md:px-5"><Story /></div></div>],
} satisfies Meta<typeof EventFeed>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 10,000 events over weeks: sticky day headings, page scroll. */
export const TenThousand: Story = {};

export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };

export const Empty: Story = { args: { events: [], empty: "Quiet kitchen. Events show up here as cooks work." } };

/** A new event on top every second, capped at 400 like the console. */
export const LiveAppend: Story = {
  render: function LiveFeed() {
    return <EventFeed events={useGrowing(makeEvents, 30)} />;
  },
};

export const Compact: Story = { args: { events: makeEvents(40), compact: true, limit: 14 } };

/** The latest 40 events with older history behind them: the footer loads 40 more each time the end comes into view. */
export const LoadOlder: Story = {
  render: function Older(args) {
    const t = usePaged((n) => makeEvents(n, 0).slice(-n), 40, 40, 400);
    return <EventFeed {...args} events={t.items} hasOlder={!t.done} loadingOlder={t.loading} onLoadOlder={t.loadMore} />;
  },
};
