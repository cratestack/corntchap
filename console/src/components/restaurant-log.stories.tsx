import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ChangeView } from "@corntchap/api";
import { RestaurantLogView } from "./restaurant-log";

const AT = Date.now();
const log: ChangeView[] = [
  { seq: 3, kind: "restaurant.decided", project: "PAY", actor: "restaurant", actor_kind: "restaurant", at: new Date(AT - 120_000).toISOString(), payload: { decision: "wake.capped", reason: "sweep", agent: "rita", misses: 3, text: "rita was woken 3 times in a row without claiming anything: the periodic sweep stops waking it until it claims a task or the work changes." } },
  { seq: 2, kind: "wake.delivered", project: "PAY", actor: "restaurant", actor_kind: "restaurant", at: new Date(AT - 600_000).toISOString(), payload: { agent: "rita", type: "claude-routine" } },
  { seq: 1, kind: "wake.failed", project: "PAY", actor: "restaurant", actor_kind: "restaurant", at: new Date(AT - 900_000).toISOString(), payload: { agent: "rita", type: "claude-routine", error: "HTTP 500" } },
];

const meta = {
  title: "Console/RestaurantLog",
  component: RestaurantLogView,
  args: { events: log },
  decorators: [(Story) => <div className="max-w-xl bg-background p-4"><Story /></div>],
} satisfies Meta<typeof RestaurantLogView>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Empty: Story = { args: { events: [] } };
export const Loading: Story = { args: { events: [], loading: true } };
export const MoreOnRequest: Story = { args: { hasMore: true } };
