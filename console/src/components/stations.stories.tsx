import type { Meta, StoryObj } from "@storybook/react-vite";
import type { StationView } from "@corntchap/api";
import { staticSource } from "@/lib/listbox";
import { CloudRunList, StationList } from "./stations";
import { StoryData } from "./story-data";
import { errorSource, loadingSource, usePagedSource } from "./ui/virtual.fixtures";

const NOW = Date.now();

const makeStations = (count: number, kind: "session" | "cloud-run"): StationView[] =>
  Array.from({ length: count }, (_, i) => ({
    id: `st_${kind}${i + 1}`,
    cook: "codex-laptop",
    project: i % 3 === 0 ? "OPS" : "PAY",
    runtime: kind === "session" ? "claude-code" : "devin",
    session: `0a1b2c3d-${String(i + 1).padStart(4, "0")}-4e5f-6a7b-8c9d0e1f2a3b`,
    directory: kind === "session" ? `/work/repos/service-${i + 1}` : null,
    repo: kind === "session" ? `acme/service-${i + 1}` : null,
    label: `Station ${i + 1}`,
    confined: false,
    main: kind === "session" && i === 0,
    kind,
    session_url: kind === "cloud-run" ? `https://app.devin.ai/sessions/${i + 1}` : null,
    ended_at: kind === "cloud-run" && i % 2 === 1 ? new Date(NOW - i * 60_000).toISOString() : null,
    target_type: kind === "cloud-run" ? "devin" : null,
    created_at: new Date(NOW - i * 3_600_000).toISOString(),
    last_woken_at: i % 2 === 0 ? new Date(NOW - i * 120_000).toISOString() : null,
    holding: i % 4 === 0 ? [`PAY-${i + 1}`] : [],
    waiting: i % 5 === 0 ? [`PAY-${i + 2}`] : [],
  }));

const sessions = makeStations(10_000, "session");
const runs = makeStations(10_000, "cloud-run");
const match = (s: StationView, q: string) => `${s.label} ${s.directory ?? ""}`.toLowerCase().includes(q.trim().toLowerCase());

const meta = {
  title: "Console/Stations",
  decorators: [
    (Story) => (
      <StoryData>
        <div className="min-h-dvh bg-background p-4 md:p-8">
          <div className="mx-auto max-w-xl">
            <Story />
          </div>
        </div>
      </StoryData>
    ),
  ],
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

/** A cook's sessions, 25 a request: the page scrolls and the next page loads as the end comes into view (the page's last list). */
export const Sessions: Story = {
  render: function Render() {
    const { source } = usePagedSource(sessions, { match });
    return <StationList source={source} cook="codex-laptop" kind="claude-code" />;
  },
};

/** A list with more below it reads on a button, never on scroll. */
export const SessionsShowMore: Story = {
  render: function Render() {
    const { source } = usePagedSource(sessions, { match, pageSize: 10 });
    return (
      <>
        <StationList source={source} cook="codex-laptop" kind="claude-code" more="button" />
        <p className="mt-6 text-[13px] text-muted-foreground">Another section below the list.</p>
      </>
    );
  },
};

export const SessionsPhone: Story = { ...Sessions, globals: { viewport: { value: "phone", isRotated: false } } };

export const SessionsLoading: Story = { render: () => <StationList source={loadingSource<StationView>()} /> };
export const SessionsEmpty: Story = { render: () => <StationList source={staticSource<StationView>([])} /> };

export const CloudRuns: Story = {
  render: function Render() {
    const { source } = usePagedSource(runs, { match });
    return <CloudRunList source={source} />;
  },
};

export const CloudRunsShowMore: Story = {
  render: function Render() {
    const { source } = usePagedSource(runs, { match, pageSize: 10 });
    return (
      <>
        <CloudRunList source={source} more="button" />
        <p className="mt-6 text-[13px] text-muted-foreground">Another section below the list.</p>
      </>
    );
  },
};

export const CloudRunsEmpty: Story = { render: () => <CloudRunList source={staticSource<StationView>([])} /> };
export const CloudRunsFailed: Story = { render: () => <CloudRunList source={errorSource<StationView>()} /> };
