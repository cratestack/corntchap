import { useMemo } from "react";
import { Link, useParams } from "react-router";
import type { ActivityView } from "@corntchap/api";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, IdChip, LabelChip, RowSkeleton } from "@/components/bits";
import { CookUsage, fmtTokens } from "@/components/usage";
import { CopyButton } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import { useProc, useProjects, useTeamChefs } from "@/lib/data";
import { fmtAgo, useNow } from "@/lib/format";
import { lastView } from "@/lib/views";
import { Crumb, Fact, Facts, Quiet, Section, linkClass } from "./common";

const activityLine = (a: ActivityView) => {
  const what = a.files.length > 0 ? `${a.files[0]}${a.files.length > 1 ? ` +${a.files.length - 1}` : ""}` : (a.summary ?? "");
  return `${a.tool}${what ? ` ${what}` : ""}`;
};

/** The one-line config for each client, as in the README ("Connecting agents: one URL per pot, no token"). */
function mcpLines(url: string) {
  return [
    {
      id: "opencode",
      title: "OpenCode",
      file: "opencode.json",
      body: `{ "mcp": { "corntchap": { "type": "remote", "url": "${url}" } } }`,
    },
    {
      id: "claude",
      title: "Claude Code",
      file: ".mcp.json",
      body: `{ "mcpServers": { "corntchap": { "type": "http", "url": "${url}" } } }`,
    },
    {
      id: "codex",
      title: "Codex",
      file: "shell",
      body: `codex mcp add corntchap --url ${url} && codex mcp login corntchap`,
    },
  ];
}

export function PotDetailsPage() {
  const { key = "" } = useParams();
  const potKey = key.toUpperCase();
  const projects = useProjects();
  const project = (projects.data ?? []).find((p) => p.key === potKey);
  const team = project?.team ?? null;
  const members = useTeamChefs(team);
  const stations = useProc("stationList", { project: potKey }, { enabled: !!project });
  const usage = useProc("usageReport", { project: potKey, days: 7 }, { enabled: !!project });
  const activity = useProc("activityList", { project: potKey, limit: 20 }, { enabled: !!project, refetchMs: 15000 });
  const now = useNow();

  const cooks = useMemo(() => (members.data ?? []).filter((m) => m.kind !== "user"), [members.data]);
  const people = useMemo(() => (members.data ?? []).filter((m) => m.kind === "user"), [members.data]);
  const mcpUrl = `${window.location.origin}/mcp/${potKey}`;

  const crumb = <Crumb to={`/pots/${potKey}`} parent={`${potKey} board`} here="About" />;

  if (projects.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Pot" title={potKey} className="pt-3">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (projects.error || !project) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Pot" title={potKey} className="pt-3">
          <EmptyState title={projects.error ? "Could not reach the pots" : "No such pot"} action={<Button asChild variant="outline"><Link to="/">Back to the kitchen</Link></Button>}>
            {projects.error ? errorMessage(projects.error) : `${potKey} is not a pot you can see.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  const counts: { id: string; label: string; n: number }[] = [
    { id: "ready", label: "Ready", n: project.ready },
    { id: "open", label: "Open", n: project.open },
    { id: "claimed", label: "Claimed", n: project.claimed },
    { id: "blocked", label: "Blocked", n: project.blocked },
    { id: "done", label: "Done", n: project.done },
    { id: "tossed", label: "Tossed", n: project.tossed },
  ];

  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Pot"
        title={project.name}
        className="pt-3"
        sub={
          <>
            <span className="rounded bg-corn px-1.5 py-px font-mono text-[12px] font-bold text-corn-foreground">{project.key}</span>
            {team && <> · team {team}</>}
          </>
        }
        actions={
          <Button asChild>
            <Link to={`/pots/${project.key}/${lastView(project.key)}`}>Open the board</Link>
          </Button>
        }
      >
        <div className="space-y-4">
          <Section title="About">
            <Facts>
              <Fact label="Name">{project.name}</Fact>
              <Fact label="Key">
                <span className="font-mono font-bold">{project.key}</span>
              </Fact>
              <Fact label="Team">
                {team ? (
                  <Link to={`/teams/${team}`} className={linkClass}>
                    {team}
                  </Link>
                ) : (
                  <span className="text-muted-foreground">none</span>
                )}
              </Fact>
              <Fact label="Lease TTL">
                <span className="font-mono">{project.lease_ttl_s >= 120 ? `${Math.round(project.lease_ttl_s / 60)} min` : `${project.lease_ttl_s} s`}</span>
              </Fact>
              <Fact label="Repository">
                {project.repo_url ? (
                  <a href={project.repo_url} target="_blank" rel="noreferrer" className={`font-mono text-[12.5px] ${linkClass}`}>
                    {project.repo_url}
                  </a>
                ) : (
                  <span className="text-muted-foreground">none set</span>
                )}
              </Fact>
              <Fact label="Questions">
                {project.open_questions > 0 ? (
                  <Link to={`/pots/${project.key}/questions`} className={linkClass}>
                    {project.open_questions} open
                  </Link>
                ) : (
                  <span className="text-muted-foreground">none open</span>
                )}
              </Fact>
            </Facts>
          </Section>

          <Section title="Tasks" aside={project.unread_pass > 0 ? <Link to={`/pots/${project.key}/pass`} className={`text-[12.5px] ${linkClass}`}>{project.unread_pass} unread on the pass</Link> : undefined}>
            <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
              {counts.map((c) => (
                <li key={c.id} className="rounded-lg border border-border bg-background px-3 py-2">
                  <p className="font-mono text-[22px] font-bold leading-none tabular-nums">{c.n}</p>
                  <p className="label-caps mt-1">{c.label}</p>
                </li>
              ))}
            </ul>
          </Section>

          <div className="grid gap-4 lg:grid-cols-2">
            <Section title="Cooks who work here" hint={team ? `The cooks of team ${team}, with their stations in ${project.key}.` : "This pot has no team, so no cook sees it."}>
              {members.isLoading ? (
                <RowSkeleton rows={2} />
              ) : cooks.length === 0 ? (
                <Quiet>No cook is in this pot's team yet.</Quiet>
              ) : (
                <ul className="divide-y divide-border">
                  {cooks.map((c) => {
                    const mine = (stations.data ?? []).filter((s) => s.cook === c.name);
                    return (
                      <li key={c.name} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                        <Avatar name={c.name} className="size-7 text-[13px]" />
                        <Link to={`/cooks/${c.name}`} className={`text-[13.5px] font-medium ${linkClass}`}>
                          {c.name}
                        </Link>
                        <span className="text-[12px] text-muted-foreground">
                          {mine.length === 0 ? "no station here" : mine.map((s) => s.label ?? s.directory ?? s.session.slice(0, 8)).join(", ")}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}
              {people.length > 0 && (
                <p className="mt-3 text-[12px] text-muted-foreground">
                  People: {people.map((p) => p.display_name ?? p.name).join(", ")}
                </p>
              )}
            </Section>

            <Section title="Usage" hint="Estimated tokens corntchap sent for this pot, 7 days.">
              {usage.data && usage.data.by_cook.length > 0 && (
                <p className="mb-1 text-[12.5px] text-muted-foreground">
                  {usage.data.by_cook.slice(0, 4).map((c) => `${c.cook} ~${fmtTokens(c.tokens)}`).join(" · ")}
                </p>
              )}
              <CookUsage report={usage.data} />
            </Section>
          </div>

          <Section title="Connect an agent" hint="One URL per pot, no token: the client signs in through your browser on first use.">
            <div className="flex items-center gap-2 rounded-lg bg-muted/50 px-3 py-2">
              <code className="min-w-0 flex-1 truncate font-mono text-[12.5px]" aria-label="MCP URL of this pot">
                {mcpUrl}
              </code>
              <CopyButton text={mcpUrl} label="Copy URL" />
            </div>
            <ul className="mt-3 space-y-3">
              {mcpLines(mcpUrl).map((s) => (
                <li key={s.id}>
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-[12.5px] font-medium">
                      {s.title} <span className="font-mono text-[11.5px] font-normal text-muted-foreground">{s.file}</span>
                    </p>
                    <CopyButton size="xs" text={s.body} />
                  </div>
                  <pre className="scrollbar-thin mt-1 overflow-x-auto rounded-lg bg-muted/50 px-3 py-2 font-mono text-[11.5px] leading-relaxed">{s.body}</pre>
                </li>
              ))}
            </ul>
          </Section>

          <Section title="Recent activity" hint="What the stations of this pot reported last.">
            {activity.isLoading ? (
              <RowSkeleton rows={2} />
            ) : (activity.data ?? []).length === 0 ? (
              <Quiet>Nothing reported yet.</Quiet>
            ) : (
              <ul className="space-y-1.5">
                {activity.data!.map((a) => (
                  <li key={`${a.station}${a.seq}`} className="flex items-baseline gap-2 text-[12px]">
                    <LabelChip className="font-mono">{a.cook}</LabelChip>
                    <span className="min-w-0 flex-1 truncate font-mono" title={a.summary ?? activityLine(a)}>
                      {activityLine(a)}
                      {a.count > 1 && <span className="text-muted-foreground"> x{a.count}</span>}
                    </span>
                    {a.task && <IdChip id={a.task} className="text-[11px]" />}
                    <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{fmtAgo(a.at, now)}</span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
      </PageShell>
    </>
  );
}
