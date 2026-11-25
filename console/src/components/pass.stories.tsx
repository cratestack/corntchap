import { useMemo } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useStoreValue } from "@/lib/use-store-value";
import { ConvList, PassFrame, type Conv } from "./pass";
import { matchString, usePagedSource } from "./ui/virtual.fixtures";
import { VirtualThread } from "./ui/virtual-thread";
import { MsgBubble, useThread, type ThreadMsg } from "./ui/virtual.fixtures";

const NOW = Date.now();

function convs(count: number): Conv[] {
  return Array.from({ length: count }, (_, i) => ({
    key: `chef-${i + 1}`,
    name: `chef-${i + 1}`,
    kind: i % 2 ? "agent" : "user",
    unread: i % 7 === 0 ? 2 : 0,
    preview: { at: new Date(NOW - i * 3_600_000).toISOString(), from: `chef-${i + 1}`, body: `A message from chef ${i + 1} about the thing they are cooking`, mine: i % 3 === 1 },
  }));
}

const pinned: Conv = { key: "pass", name: "pass", channel: true, kind: "channel", unread: 0, preview: null };

/** The conversation list as the server pages and searches it. */
function List({ chefs }: { chefs: number }) {
  const all = useMemo(() => convs(chefs), [chefs]);
  const found = usePagedSource(all, { pageSize: 25, match: (c, q) => matchString(c.name, q) });
  const unread = useStoreValue(false);
  return (
    <ConvList
      active="pass"
      select={() => undefined}
      pinned={pinned}
      source={found.source}
      q={found.query}
      onQ={found.onQuery}
      unread={unread.value}
      onUnread={unread.set}
      compact={false}
      now={NOW}
    />
  );
}

function Frame({ chefs }: { chefs: number }) {
  const thread = useThread({ total: 300, loaded: 60 });
  return (
    <div className="min-h-dvh bg-background px-4 pt-6 pb-8 md:px-5 lg:px-8">
      <h1 className="mb-6 text-[28px]">PAY</h1>
      <PassFrame
        compact={false}
        aside={<List chefs={chefs} />}
      >
        <section aria-label="The pass" className="flex min-h-0 min-w-0 flex-1 flex-col md:sticky md:top-5 md:h-[var(--pass-h,40rem)]">
          <header className="flex min-h-14 shrink-0 items-center border-b border-border bg-card px-4 font-display text-[17px]">pass</header>
          <div role="log" aria-label="Messages" className="relative min-h-0 flex-1">
            <VirtualThread<ThreadMsg>
              items={thread.items}
              itemKey={(m) => m.id}
              lastSeq={thread.lastSeq}
              hasOlder={thread.hasOlder}
              loadingOlder={thread.loadingOlder}
              onLoadOlder={thread.loadOlder}
              renderItem={(m) => <MsgBubble msg={m} />}
              itemClassName="px-3"
              aria-label="Messages"
            />
          </div>
          <div className="shrink-0 border-t border-border bg-card p-3">
            <input aria-label="Message" placeholder="Write to the pass" className="h-10 w-full rounded-xl border border-input bg-background px-3 text-[14px]" />
          </div>
        </section>
      </PassFrame>
    </div>
  );
}

const meta = {
  title: "Console/PassFrame",
  component: Frame,
  args: { chefs: 6 },
} satisfies Meta<typeof Frame>;
export default meta;
type Story = StoryObj<typeof meta>;

/** Tablet and up: the conversation list runs down the page, the thread is a chat pane beside it that scrolls inside itself. */
export const Default: Story = {};

/** A long conversation list is carried by the page; the thread stays in view while it scrolls. */
export const ManyConversations: Story = { args: { chefs: 60 } };

/** Ten thousand conversations: paged and virtualized, the page carries the list. */
export const TenThousandConversations: Story = { args: { chefs: 10_000 } };

/** Phone: a chat screen fixed between the top bar and the tab bar. */
export const Phone: Story = { globals: { viewport: { value: "phone", isRotated: false } } };
