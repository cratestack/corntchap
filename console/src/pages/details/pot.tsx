import { useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { ChevronRight, Pencil, Trash2 } from "lucide-react";
import type { ActivityView } from "@corntchap/api";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, Group, GroupRow, IdChip, LabelChip, RowSkeleton } from "@/components/bits";
import { PotDeleteDialog, PotRenameDialog } from "@/components/pot-options";
import { UsagePanel } from "@/components/usage";
import { CodeBlock, CopyButton, CopyLine } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import { useProc, useProjects, useTeamChefs, useV1, type TeamRow } from "@/lib/data";
import { useSession } from "@/lib/session";
import { fmtAgo, useNow } from "@/lib/format";
import { lastView } from "@/lib/views";
import { PotRepos } from "@/components/repos";
import { PotGuide } from "@/components/pot-guide";
import { Crumb, Quiet, linkClass } from "./common";

const activityLine = (a: ActivityView) => {
  const what = a.files.length > 0 ? `${a.files[0]}${a.files.length > 1 ? ` +${a.files.length - 1}` : ""}` : (a.summary ?? "");
  return `${a.tool}${what ? ` ${what}` : ""}`;
};

/**
 * The one-line config for each client, as in the README ("Connecting agents: one URL per pot, no token"). The
 * server is named after the pot (corntchap-pay), so a repo can carry several pots' servers and each keeps its sign-in.
 */
function mcpLines(url: string, key: string) {
  const name = `corntchap-${key.toLowerCase()}`;
  return [
    {
      id: "opencode",
      title: "OpenCode",
      file: "opencode.json",
      body: `{ "mcp": { "${name}": { "type": "remote", "url": "${url}" } } }`,
    },
    {
      id: "claude",
      title: "Claude Code",
      file: ".mcp.json",
      body: `{ "mcpServers": { "${name}": { "type": "http", "url": "${url}" } } }`,
    },
    {
      id: "codex",
      title: "Codex",
      file: "shell",
      body: `codex mcp add ${name} --url ${url} && codex mcp login ${name}`,
    },
    {
      id: "copilot",
      title: "Copilot (VS Code)",
      file: ".vscode/mcp.json",
      body: `{ "servers": { "${name}": { "type": "http", "url": "${url}" } } }`,
    },
    {
      id: "copilot-cli",
      title: "Copilot CLI",
      file: "~/.copilot/mcp-config.json",
      body: `{ "mcpServers": { "${name}": { "type": "http", "url": "${url}", "tools": ["*"] } } }`,
    },
    {
      id: "antigravity",
      title: "Antigravity",
      file: ".agents/mcp_config.json",
      body: `{ "mcpServers": { "${name}": { "serverUrl": "${url}" } } }`,
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
  const me = useSession();
  const teamsQ = useV1<TeamRow[]>("/teams");
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<"rename" | "delete" | null>(null);
  const canManage = me.is_admin || teamsQ.data?.find((t) => t.name === team)?.role === "owner";

  const cooks = useMemo(() => (members.data ?? []).filter((m) => m.kind !== "user"), [members.data]);
  const people = useMemo(() => (members.data ?? []).filter((m) => m.kind === "user"), [members.data]);
  const mcpUrl = `${window.location.origin}/mcp/${potKey}`;

  const crumb = <Crumb to={`/pots/${potKey}`} parent={`${potKey} board`} here="About" />;

  if (projects.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Pot" title={potKey} className="pt-2">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (projects.error || !project) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Pot" title={potKey} className="pt-2">
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
        className="pt-2 sm:pt-3"
        sub={
          <>
            <span className="rounded bg-muted px-1.5 py-px font-mono text-[12px] font-bold text-foreground">{project.key}</span>
            {team && <> · team {team}</>}
          </>
        }
        actions={
          // phones reach the board from the bottom tabs; the button only pushed the header onto two rows
          <Button asChild className="max-md:hidden">
            <Link to={`/pots/${project.key}/${lastView(project.key)}`}>Open the board</Link>
          </Button>
        }
      >
        <div className="space-y-6">
          <Group
            title="Tasks"
            aside={
              project.unread_pass > 0 ? (
                <Link to={`/pots/${project.key}/pass`} className={`text-[12px] ${linkClass}`}>
                  {project.unread_pass} unread on the pass
                </Link>
              ) : undefined
            }
          >
            <ul className="grid grid-cols-3 gap-px bg-border sm:grid-cols-6">
              {counts.map((c) => (
                <li key={c.id} className="bg-card px-3 py-2.5 sm:py-3">
                  <p className={`font-mono text-[20px] leading-none font-bold tabular-nums ${c.n === 0 ? "text-muted-foreground" : ""}`}>{c.n}</p>
                  <p className="label-caps mt-1.5">{c.label}</p>
                </li>
              ))}
            </ul>
          </Group>

          <PotGuide project={project} canManage={canManage} />

          <PotRepos project={project} canManage={canManage} />

          <div className="space-y-6 lg:grid lg:grid-cols-2 lg:items-start lg:gap-x-6 lg:space-y-0 lg:gap-y-6">
            <div className="space-y-6">
            <Group title="Details">
              <GroupRow label="Team" value={team ?? "none"} {...(team ? { to: `/teams/${team}` } : {})} />
              <GroupRow label="Lease" hint="How long a scooped task stays held without a heartbeat" value={<span className="font-mono">{project.lease_ttl_s >= 120 ? `${Math.round(project.lease_ttl_s / 60)} min` : `${project.lease_ttl_s} s`}</span>} />
              <GroupRow
                label="Questions"
                value={project.open_questions > 0 ? `${project.open_questions} open` : "none open"}
                {...(project.open_questions > 0 ? { to: `/pots/${project.key}/questions` } : {})}
              />
            </Group>

            <Group title="Cooks who work here" hint={team ? `The cooks of team ${team}, with their stations in ${project.key}.` : "This pot has no team, so no cook sees it."}>
              {members.isLoading ? (
                <div className="p-3">
                  <RowSkeleton rows={2} />
                </div>
              ) : cooks.length === 0 ? (
                <GroupRow label="No cook yet" hint="No cook is in this pot's team." />
              ) : (
                cooks.map((c) => {
                  const mine = (stations.data ?? []).filter((s) => s.cook === c.name);
                  return (
                    <Link
                      key={c.name}
                      to={`/cooks/${c.name}`}
                      className="flex min-h-14 items-center gap-3 px-4 py-2.5 outline-none transition-colors hover:bg-muted/50 focus-visible:bg-muted/60 active:bg-muted"
                    >
                      <Avatar name={c.name} className="size-8 rounded-lg text-[14px]" />
                      <span className="min-w-0 flex-1">
                        <span className="block text-[14px] font-medium leading-tight">{c.name}</span>
                        <span className="block truncate text-[12px] text-muted-foreground">
                          {mine.length === 0 ? "no station here" : mine.map((s) => s.label ?? s.directory ?? s.session.slice(0, 8)).join(", ")}
                        </span>
                      </span>
                      <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
                    </Link>
                  );
                })
              )}
              {people.length > 0 && <GroupRow label="People" value={people.map((p) => p.display_name ?? p.name).join(", ")} />}
            </Group>

            </div>

            <Group title="Usage" hint="Estimated tokens corntchap sent for this pot.">
              <div className="p-3.5">
                <UsagePanel report={usage.data} showCooks />
              </div>
            </Group>
          </div>

          <Group title="Connect an agent" hint="One URL per pot, no token: the client signs in through your browser on first use.">
            <div className="space-y-4 p-3.5">
              <CopyLine text={mcpUrl} label="Copy URL" />
              <ul className="space-y-4">
                {mcpLines(mcpUrl, key).map((s) => (
                  <li key={s.id} className="min-w-0">
                    <p className="text-[13px] font-medium">
                      {s.title} <span className="font-mono text-[11.5px] font-normal text-muted-foreground">{s.file}</span>
                    </p>
                    <CodeBlock label={`${s.title} configuration`} className="mt-1.5">
                      {s.body}
                    </CodeBlock>
                    <CopyButton text={s.body} label={`Copy ${s.title}`} className="mt-2 max-sm:h-11 max-sm:w-full" />
                  </li>
                ))}
              </ul>
            </div>
          </Group>

          <Group title="Recent activity" hint="What the stations of this pot reported last.">
            <div className="p-3.5">
              {activity.isLoading ? (
                <RowSkeleton rows={2} />
              ) : (activity.data ?? []).length === 0 ? (
                <Quiet>Nothing reported yet.</Quiet>
              ) : (
                <ul className="space-y-2">
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
            </div>
          </Group>

          {canManage && (
            <Group title="Danger zone" tone="danger" className="lg:max-w-xl">
              <GroupRow icon={<Pencil />} label="Rename" hint="The display name only: the key stays" onClick={() => setDialog("rename")} />
              <GroupRow icon={<Trash2 />} label="Delete" hint="The pot and everything in it, for good" destructive onClick={() => setDialog("delete")} />
            </Group>
          )}
        </div>
      </PageShell>
      <PotRenameDialog pot={project} open={dialog === "rename"} onOpenChange={(o) => !o && setDialog(null)} />
      <PotDeleteDialog pot={project} open={dialog === "delete"} onOpenChange={(o) => !o && setDialog(null)} onDeleted={() => void navigate("/", { replace: true })} />
    </>
  );
}
