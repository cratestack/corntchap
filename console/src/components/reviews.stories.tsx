import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReviewView } from "@corntchap/api";
import { staticSource } from "@/lib/listbox";
import { ReviewsList } from "./reviews";
import { StoryData } from "./story-data";
import { loadingSource, usePagedSource } from "./ui/virtual.fixtures";

const NOW = Date.now();

const STATES = ["requested", "approved", "changes_requested", "commented", "cancelled"] as const;

const reviews: ReviewView[] = Array.from({ length: 10_000 }, (_, i) => ({
  id: `PAY-R${i + 1}`,
  project: "PAY",
  task: `PAY-${(i % 400) + 1}`,
  task_title: `Retry the refund webhook on a timeout (${i + 1})`,
  round: (i % 3) + 1,
  state: STATES[i % STATES.length]!,
  requested_by: "codex-laptop",
  reviewer: i % 2 === 0 ? "bob" : null,
  reviewed_by: i % 5 === 0 || i % 5 === 4 ? null : "bob",
  note: i % 3 === 0 ? "Ready to look at: the retry is capped at five attempts." : "",
  body: i % 5 === 1 ? "Looks right. One nit about the backoff constant." : null,
  created_at: new Date(NOW - i * 3_600_000).toISOString(),
  decided_at: null,
  updated_at: new Date(NOW - i * 60_000).toISOString(),
}));
const match = (r: ReviewView, q: string) => `${r.id} ${r.task} ${r.task_title}`.toLowerCase().includes(q.trim().toLowerCase());

const meta = {
  title: "Console/ReviewsList",
  decorators: [
    (Story) => (
      <StoryData>
        <div className="min-h-dvh bg-background p-4 md:p-8">
          <div className="mx-auto max-w-3xl">
            <Story />
          </div>
        </div>
      </StoryData>
    ),
  ],
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

/** 10,000 reviews, 25 a request: the page scrolls and the next page loads as the end comes into view. */
export const TenThousand: Story = {
  render: function Render() {
    const { source } = usePagedSource(reviews, { match });
    return <ReviewsList source={source} />;
  },
};

/** Above another list (the questions): it reads its next page on a button. */
export const ShowMore: Story = {
  render: function Render() {
    const { source } = usePagedSource(reviews, { match, pageSize: 10 });
    return (
      <>
        <ReviewsList source={source} more="button" />
        <p className="mt-6 text-[13px] text-muted-foreground">The questions list is below.</p>
      </>
    );
  },
};

export const Phone: Story = { ...TenThousand, globals: { viewport: { value: "phone", isRotated: false } } };
export const Loading: Story = { render: () => <ReviewsList source={loadingSource<ReviewView>()} /> };
export const Empty: Story = { render: () => <ReviewsList source={staticSource<ReviewView>([])} /> };
