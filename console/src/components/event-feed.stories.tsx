import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ChangeView } from "@corntchap/api";
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

const AT = Date.now();
const ago = (minutes: number) => new Date(AT - minutes * 60_000).toISOString();
let seq = 1;
/** An event of the restaurant (the server itself): its actor is the restaurant, whatever the kind. */
const byRestaurant = (kind: string, minutes: number, payload: ChangeView["payload"], task?: string): ChangeView => ({
  seq: seq++,
  kind,
  project: "PAY",
  ...(task ? { task } : {}),
  actor: "restaurant",
  actor_kind: "restaurant",
  at: ago(minutes),
  payload,
});

/** Every decision the server takes by itself, as the feed shows it (oldest first, like the live feed). */
const decisions: ChangeView[] = [
  byRestaurant("task.lease_expired", 90, { agent: "ada", reason: "stalled", why: "the lease ended: no progress for 2 hours" }, "PAY-4"),
  byRestaurant("task.lease_expired", 85, { agent: "bola", reason: "max_hold", why: "the lease ended: the task was held for more than 8 hours" }, "PAY-5"),
  byRestaurant("task.blocked", 80, { reason: "bounced 5×: needs a person", why: "5 give-backs by cooks with nobody touching the task in between: it is work a cook cannot do, so a person must look" }, "PAY-6"),
  byRestaurant("task.completed", 75, { result: "Merged https://github.com/acme/shop/pull/12", reason: "merged", why: "its pull request merged and nothing else is going on: no other open pull request, subtask or unfinished dependency" }, "PAY-7"),
  byRestaurant("forge.linked", 70, { repo: "acme/shop", number: 12, via: "branch", reason: "auto_link", why: "the pull request's branch names the task" }, "PAY-7"),
  byRestaurant("forge.backfill", 65, { repo: "acme/shop", reason: "backfill", why: "a repository joined the pot: its open pull requests were searched for ones that name the pot's tasks" }),
  byRestaurant("wake.failed", 60, { agent: "rita", reason: "HTTP 500 from the routine endpoint" }),
  byRestaurant("restaurant.decided", 55, { decision: "wake.held", reason: "backoff", agent: "rita", text: "rita was woken a moment ago and has not claimed anything yet (2 wakes in a row): the next wake waits until 14:05 UTC.", misses: 2 }),
  byRestaurant("restaurant.decided", 50, { decision: "wake.held", reason: "retry_after", agent: "rita", text: "rita's cloud target asked to wait (it answered 429): no wake before 14:20 UTC." }),
  byRestaurant("restaurant.decided", 45, { decision: "wake.held", reason: "busy", agent: "ada", text: "ada holds a lease right now: its session is already running, so it is not woken." }),
  byRestaurant("restaurant.decided", 40, { decision: "wake.held", reason: "stations_busy", agent: "bola", text: "All of bola's sessions here are busy: no wake is sent, they pick the work up themselves." }),
  byRestaurant("restaurant.decided", 35, { decision: "wake.capped", reason: "sweep", agent: "rita", misses: 3, text: "rita was woken 3 times in a row without claiming anything: the periodic sweep stops waking it until it claims a task or the work changes." }),
  byRestaurant("restaurant.decided", 30, { decision: "wake.withheld", reason: "fire_and_forget", agent: "rita", repeated: 4, text: "rita was not woken (comment): it is a fire-and-forget cook, which only takes ready work and answers its owner's direct messages." }, "PAY-8"),
  byRestaurant("restaurant.decided", 25, { decision: "wake.withheld", reason: "uncovered", agent: "rita", text: "rita was not woken (forge): none of its cloud targets serves PAY, so there is nowhere to start a run." }, "PAY-9"),
  byRestaurant("restaurant.decided", 20, { decision: "claim.skipping", reason: "bounce", agent: "ada", text: "ada gave PAY-10 back 3 times with nobody touching it: claim_next skips it for ada until someone comments, edits it or answers." }, "PAY-10"),
  byRestaurant("restaurant.decided", 15, { decision: "catch_up", reason: "away", agent: "bola", text: "bola was away 3 hours: its first claim in PAY returned a briefing instead of a lease." }),
  byRestaurant("restaurant.decided", 10, { decision: "merge.kept_open", reason: "open_pr", text: "acme/shop#14 merged, but PAY-11 stays as it is: another pull request linked to it is still open." }, "PAY-11"),
  byRestaurant("restaurant.decided", 5, { decision: "run.ended", reason: "stale", agent: "rita", station: "st_4f9a1c2b", text: "rita's cloud run st_4f9a1c2b was ended: it held nothing and said nothing for 2 hours." }),
  byRestaurant("restaurant.decided", 2, { decision: "role.kept", reason: "last_super_admin", agent: "octo", text: "The identity provider no longer grants octo the admin role, but they are the last usable super-admin: the restaurant kept super_admin so the instance is not locked out." }),
];

/** The restaurant's decisions, one of each: the saffron tile with a chef's hat, the reason, and a (×n) when it repeated. */
export const RestaurantDecisions: Story = { args: { events: decisions } };

export const RestaurantDecisionsCompact: Story = { args: { events: decisions, compact: true } };

/** A paged read of the activity (`eventPage`) gives newest first, with older pages behind: the footer loads 40 more each time the end comes into view. */
export const NewestFirstPaged: Story = {
  render: function Newest(args) {
    const t = usePaged((n) => makeEvents(n, 0).slice(-n).reverse(), 40, 40, 400);
    return <EventFeed {...args} events={t.items} newestFirst hasOlder={!t.done} loadingOlder={t.loading} onLoadOlder={t.loadMore} />;
  },
};
