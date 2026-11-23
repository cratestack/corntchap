import { useMemo } from "react";
import { Link, useParams } from "react-router";
import { useQueries } from "@tanstack/react-query";
import { MessageCircle } from "lucide-react";
import type { ActivityView } from "@corntchap/api";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, IdChip, LabelChip, RowSkeleton, StateDot, STATE_LABEL, cookState } from "@/components/bits";
import { StationList } from "@/components/stations";
import { CookUsage } from "@/components/usage";
import { TaskRow } from "@/components/task-views";
import { WakeTargets } from "@/pages/cooks";
import { Button } from "@/components/ui/button";
import { client, errorMessage } from "@/lib/api";
import { useAgents, useClaimedTasks, useProc, useProjects } from "@/lib/data";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useLive } from "@/lib/live";
import { useOpenTask } from "@/lib/nav";
import { KIND_LABEL } from "@/lib/snippets";
import { Crumb, Fact, Facts, PotLink, Quiet, Section, linkClass } from "./common";

const activityLine = (a: ActivityView) => {
  const what = a.files.length > 0 ? `${a.files[0]}${a.files.length > 1 ? ` +${a.files.length - 1}` : ""}` : (a.summary ?? "");
  return `${a.tool}${what ? ` ${what}` : ""}`;
};

export function CookDetailsPage() {
  const { name = "" } = useParams();
  const agents = useAgents();
  const projects = useProjects();
  const stationsQ = useProc("stationList", { cook: name });
  const usage = useProc("usageReport", { cook: name, days: 7 });
  const claimed = useClaimedTasks(projects.data);
  const { events } = useLive();
  const openTask = useOpenTask();
  const now = useNow();

  const cook = (agents.data ?? []).find((a) => a.name.toLowerCase() === name.toLowerCase());
  const stations = useMemo(() => stationsQ.data ?? [], [stationsQ.data]);

  const myTeams = useMemo(() => cook?.teams ?? [], [cook]);
  const pots = useMemo(() => (projects.data ?? []).filter((p) => p.team && myTeams.includes(p.team)), [projects.data, myTeams]);
  const held = useMemo(
    () => claimed.tasks.filter((t) => t.claimed_by?.toLowerCase() === name.toLowerCase() && t.lease_live),
    [claimed.tasks, name],
  );
  // a finished task forgets who held it, so the recent completions come from the event feed
  const served = useMemo(() => {
    const out: { task: string; at: string }[] = [];
    for (const e of [...events].reverse()) {
      if (e.kind === "task.completed" && e.actor?.toLowerCase() === name.toLowerCase() && e.task) out.push({ task: e.task, at: e.at });
      if (out.length >= 8) break;
    }
    return out;
  }, [events, name]);

  const activity = useQueries({
    queries: stations.map((s) => ({
      queryKey: ["data", "activity", s.project, s.id],
      queryFn: () => client.procedures.activityList({ project: s.project, station: s.id, limit: 10 }),
      staleTime: 1500,
      refetchInterval: 15000,
    })),
  });
  const recent = useMemo(() => {
    const rows: (ActivityView & { project: string })[] = [];
    activity.forEach((r, i) => {
      for (const a of r.data ?? []) rows.push({ ...a, project: stations[i]!.project });
    });
    return rows.sort((a, b) => b.at.localeCompare(a.at)).slice(0, 15);
  }, [activity, stations]);

  const crumb = <Crumb to="/cooks" parent="Cooks" here={name} />;

  if (agents.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Cook" title={name} className="pt-3">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (agents.error || !cook) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Cook" title={name} className="pt-3">
          <EmptyState title={agents.error ? "Could not reach the cooks" : "No such cook"} action={<Button asChild variant="outline"><Link to="/cooks">Back to Cooks</Link></Button>}>
            {agents.error ? errorMessage(agents.error) : `${name} is not a cook you can see.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  const st = cookState(cook, now);
  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Cook"
        title={cook.name}
        className="pt-3"
        sub={
          <>
            {KIND_LABEL[cook.agent_kind ?? "other"] ?? cook.agent_kind ?? "Agent"} · {cook.runtime ?? "local"}
            {cook.owner && <> · owned by {cook.owner}</>}
          </>
        }
        actions={
          <Button asChild variant="outline">
            <Link to={`/cooks#${cook.name}`}>Manage on Cooks</Link>
          </Button>
        }
      >
        <div className="space-y-4">
          <Section title="At a glance">
            <div className="flex items-start gap-4">
              <Avatar name={cook.name} className="size-12 rounded-lg text-[22px]" />
              <Facts>
                <Fact label="State">
                  <span className="inline-flex items-center gap-1.5">
                    <StateDot state={st} /> {STATE_LABEL[st]}
                  </span>
                </Fact>
                <Fact label="Last seen">{fmtAgo(cook.last_seen_at, now)}</Fact>
                <Fact label="Kind">
                  {KIND_LABEL[cook.agent_kind ?? "other"] ?? cook.agent_kind ?? "other"} · {cook.runtime ?? "local"}
                </Fact>
                <Fact label="Owner">{cook.owner ?? <span className="text-muted-foreground">none</span>}</Fact>
                <Fact label="Leases">
                  <span className="font-mono">
                    {cook.busy} of {cook.max_leases}
                  </span>{" "}
                  <span className="text-muted-foreground">in use</span>
                </Fact>
                <Fact label="May also">
                  {(cook.grants ?? []).length > 0 ? (cook.grants ?? []).join(", ") : <span className="text-muted-foreground">works its tasks only</span>}
                </Fact>
              </Facts>
            </div>
          </Section>

          <div className="grid gap-4 lg:grid-cols-2">
            <Section title="Teams and pots" hint="The teams decide which pots this cook sees.">
              {myTeams.length === 0 ? (
                <Quiet>In no team: it sees no pot.</Quiet>
              ) : (
                <ul className="space-y-2">
                  {myTeams.map((t) => {
                    const tp = pots.filter((p) => p.team === t);
                    return (
                      <li key={t} className="flex flex-wrap items-center gap-x-3 gap-y-1">
                        <Link to={`/teams/${t}`} className={`text-[14px] font-medium ${linkClass}`}>
                          {t}
                        </Link>
                        {tp.length === 0 ? (
                          <span className="text-[12px] text-muted-foreground">no pots</span>
                        ) : (
                          tp.map((p) => (
                            <span key={p.key} className="inline-flex items-center gap-1.5">
                              <PotLink pot={p.key} />
                              <span className="text-[12.5px] text-muted-foreground">{p.name}</span>
                            </span>
                          ))
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
            </Section>

            <Section title="Wake targets" hint="How an idle cook is started when work arrives.">
              <WakeTargets agent={cook.name} runtime={cook.runtime} />
            </Section>
          </div>

          <Section title="Stations" hint="Registered agent sessions of this cook, one per folder and pot.">
            {stationsQ.isLoading ? <RowSkeleton rows={2} /> : <StationList stations={stations} cook={cook.name} kind={cook.agent_kind} />}
          </Section>

          <div className="grid gap-4 lg:grid-cols-2">
            <Section title="Tasks" hint="Held right now, and served lately.">
              {claimed.isLoading ? (
                <RowSkeleton rows={2} />
              ) : held.length === 0 && served.length === 0 ? (
                <Quiet>Nothing held and nothing served in this session's feed.</Quiet>
              ) : (
                <div className="space-y-2">
                  {held.map((t) => (
                    <TaskRow key={t.id} task={t} selected={false} onOpen={openTask} />
                  ))}
                  {served.length > 0 && (
                    <>
                      <p className="label-caps pt-2">Recently served</p>
                      <ul className="divide-y divide-border">
                        {served.map((s) => (
                          <li key={`${s.task}${s.at}`} className="flex items-center justify-between gap-3 py-1.5">
                            <button type="button" onClick={() => openTask(s.task)} className={`font-mono text-[12px] font-bold ${linkClass}`}>
                              {s.task}
                            </button>
                            <span className="font-mono text-[10.5px] text-muted-foreground">{fmtAgo(s.at, now)}</span>
                          </li>
                        ))}
                      </ul>
                    </>
                  )}
                </div>
              )}
            </Section>

            <Section title="Usage" hint="Estimated tokens corntchap sent this cook.">
              <CookUsage report={usage.data} />
            </Section>
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <Section title="Recent activity" hint="What its stations' hooks reported last.">
              {stations.length === 0 ? (
                <Quiet>No stations, so no activity is reported.</Quiet>
              ) : activity.some((r) => r.isLoading) && recent.length === 0 ? (
                <RowSkeleton rows={2} />
              ) : recent.length === 0 ? (
                <Quiet>Nothing reported yet.</Quiet>
              ) : (
                <ul className="space-y-1.5">
                  {recent.map((a) => (
                    <li key={`${a.project}${a.station}${a.seq}`} className="flex items-baseline gap-2 text-[12px]">
                      <LabelChip className="font-mono">{a.project}</LabelChip>
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

            <Section title="Pass" hint="Direct conversations with this cook, per pot.">
              {pots.length === 0 ? (
                <Quiet>No pots, so no conversations.</Quiet>
              ) : (
                <ul className="space-y-1.5">
                  {pots.map((p) => (
                    <li key={p.key}>
                      <Link to={`/pots/${p.key}/pass?with=${encodeURIComponent(cook.name)}`} className={`inline-flex items-center gap-2 text-[13px] ${linkClass}`}>
                        <MessageCircle className="size-3.5 text-muted-foreground" aria-hidden="true" />
                        Talk with {cook.name} in <span className="font-mono font-bold">{p.key}</span>
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
              <p className="mt-3 text-[11.5px] text-muted-foreground">{plural(pots.length, "pot")} visible to {cook.name}.</p>
            </Section>
          </div>
        </div>
      </PageShell>
    </>
  );
}
