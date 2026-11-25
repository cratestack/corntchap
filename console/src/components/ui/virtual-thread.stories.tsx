import type { Meta, StoryObj } from "@storybook/react-vite";
import { VirtualThread } from "./virtual-thread";
import { MsgBubble, useThread, type ThreadMsg } from "./virtual.fixtures";

const meta = {
  title: "UI/VirtualThread",
  component: VirtualThread<ThreadMsg>,
  decorators: [(Story) => <div className="h-dvh bg-background p-4"><div className="mx-auto h-full max-w-xl overflow-hidden rounded-[14px] border border-border"><Story /></div></div>],
} satisfies Meta<typeof VirtualThread<ThreadMsg>>;
export default meta;
type Story = StoryObj<typeof meta>;

const common = {
  itemKey: (m: ThreadMsg) => m.id,
  renderItem: (m: ThreadMsg) => <MsgBubble msg={m} />,
  itemClassName: "px-3",
  "aria-label": "Messages",
} as const;

/** 10,000 messages of 1 to 5 lines; the newest 100 are loaded and scrolling to the top loads 50 older ones, keeping your place. */
export const TenThousandMessages: Story = {
  args: { ...common, items: [], lastSeq: 0 },
  render: function Render1(args) {
    const t = useThread({ total: 10_000, loaded: 100 });
    return <VirtualThread {...args} items={t.items} lastSeq={t.lastSeq} hasOlder={t.hasOlder} loadingOlder={t.loadingOlder} onLoadOlder={t.loadOlder} />;
  },
};

/** A new message arrives every second: followed while you are at the bottom, a "New messages" pill when you have scrolled up. */
export const LiveAppend: Story = {
  args: { ...common, items: [], lastSeq: 0 },
  render: function Render2(args) {
    const t = useThread({ total: 300, loaded: 60, appendMs: 1000 });
    return <VirtualThread {...args} items={t.items} lastSeq={t.lastSeq} hasOlder={t.hasOlder} loadingOlder={t.loadingOlder} onLoadOlder={t.loadOlder} />;
  },
};

/** A short thread sits at the bottom of the pane. */
export const FewMessages: Story = {
  args: { ...common, items: [], lastSeq: 0 },
  render: function Render3(args) {
    const t = useThread({ total: 3, loaded: 3 });
    return <VirtualThread {...args} items={t.items} lastSeq={t.lastSeq} />;
  },
};

export const Empty: Story = {
  args: { ...common, items: [], lastSeq: 0, empty: <p className="grid h-full place-items-center text-muted-foreground">A quiet channel.</p> },
};

export const Phone: Story = {
  ...TenThousandMessages,
  globals: { viewport: { value: "phone", isRotated: false } },
};
