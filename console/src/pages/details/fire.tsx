import { useMemo } from "react";
import { Link, useParams } from "react-router";
import { Flame } from "lucide-react";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, LabelChip, RowSkeleton, StateDot, cookState } from "@/components/bits";
import { CopyButton } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import { useAgents, useV1, type AgentRow } from "@/lib/data";
import { fmtAgo, useNow } from "@/lib/format";
import { useLive } from "@/lib/live";
import { Crumb, Fact, Facts, PotLink, Quiet, Section, linkClass, useWakeTargetsOf } from "./common";

export function FireDetailsPage() {
  const { name = "" } = useParams();
  const relays = useV1<AgentRow[]>("/relays");
  const agents = useAgents();
  const { events } = useLive();
  const now = useNow();

  const fire = (relays.data ?? []).find((r) => r.name.toLowerCase() === name.toLowerCase());
  // the cooks a fire may wake are its owner's
  const owned = useMemo(() => (agents.data ?? []).filter((a) => !!fire?.owner && a.owner === fire.owner), [agents.data, fire]);
  const wake = useWakeTargetsOf(useMemo(() => owned.map((a) => a.name), [owned]));
  const wakes = owned.filter((a) => (wake.byCook.get(a.name) ?? []).some((w) => w.type === "relay"));

  // `run.started` / `run.finished` are what the relay reports for each agent it starts
  const runs = useMemo(() => {
    const names = new Set(wakes.map((a) => a.name.toLowerCase()));
    return [...events]
      .reverse()
      .filter((e) => e.kind.startsWith("run.") && names.has((e.payload.agent ?? e.actor ?? "").toLowerCase()))
      .slice(0, 12);
  }, [events, wakes]);

  const crumb = <Crumb to="/fires" parent="Fires" here={name} />;

  if (relays.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Fire" title={name} className="pt-3">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (relays.error || !fire) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Fire" title={name} className="pt-3">
          <EmptyState title={relays.error ? "Could not reach the fires" : "No such fire"} action={<Button asChild variant="outline"><Link to="/fires">Back to Fires</Link></Button>}>
            {relays.error ? errorMessage(relays.error) : `${name} is not a fire you can see.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  // same rule as the sidebar's fire card: awake within five minutes and not put out
  const st = cookState({ ...fire, busy: 0 }, now);
  const lit = st === "awake";
  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Fire"
        title={fire.name}
        className="pt-3"
        sub="A relay running on a machine of its owner: it listens for work and wakes that owner's local cooks."
        actions={
          <Button asChild variant="outline">
            <Link to="/fires">Manage on Fires</Link>
          </Button>
        }
      >
        <div className="space-y-4">
          <Section title="At a glance">
            <div className="flex items-start gap-4">
              <span className={`grid size-12 shrink-0 place-items-center rounded-lg ${lit ? "bg-corn/20" : "bg-muted"}`}>
                <Flame className={`size-6 ${lit ? "text-corn-text" : "text-muted-foreground"}`} aria-hidden="true" />
              </span>
              <Facts>
                <Fact label="State">
                  <span className="inline-flex items-center gap-1.5">
                    <StateDot state={lit ? "cooking" : st} className={lit ? "" : "bg-muted-foreground/50"} />
                    {lit ? "lit" : fire.disabled ? "put out" : "stale, banked"}
                  </span>
                </Fact>
                <Fact label="Last seen">{fmtAgo(fire.last_seen_at, now)}</Fact>
                <Fact label="Owner">{fire.owner ?? <span className="text-muted-foreground">none</span>}</Fact>
                <Fact label="Enroll">
                  <span className="inline-flex flex-wrap items-center gap-2">
                    <code className="font-mono text-[11.5px]">corntchap-cli fire enroll {fire.name}</code>
                    <CopyButton size="xs" text={`corntchap-cli fire enroll ${fire.name}`} />
                  </span>
                </Fact>
              </Facts>
            </div>
          </Section>

          <Section title="Cooks it wakes" hint="The owner's cooks that have a relay wake target.">
            {agents.isLoading || wake.isLoading ? (
              <RowSkeleton rows={2} />
            ) : wakes.length === 0 ? (
              <Quiet>No cook of {fire.owner ?? "its owner"} has a relay wake target. Add one on the cook, or run corntchap-cli cook setup.</Quiet>
            ) : (
              <ul className="divide-y divide-border">
                {wakes.map((a) => {
                  const cs = cookState(a, now);
                  return (
                    <li key={a.name} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                      <Avatar name={a.name} className="size-7 text-[13px]" />
                      <Link to={`/cooks/${a.name}`} className={`text-[13.5px] font-medium ${linkClass}`}>
                        {a.name}
                      </Link>
                      <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
                        <StateDot state={cs} /> {cs}
                      </span>
                      {(a.teams ?? []).length > 0 && (
                        <span className="flex flex-wrap gap-1">
                          {(a.teams ?? []).map((t) => (
                            <LabelChip key={t}>{t}</LabelChip>
                          ))}
                        </span>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </Section>

          <Section title="Recent runs" hint="Each time the relay started one of these cooks and when it finished.">
            {runs.length === 0 ? (
              <Quiet>No run reported since this page's feed began.</Quiet>
            ) : (
              <ul className="divide-y divide-border">
                {runs.map((e) => {
                  const agent = e.payload.agent ?? e.actor ?? "";
                  const code = (e.payload as { exit_code?: number | null }).exit_code;
                  const started = e.kind === "run.started";
                  return (
                    <li key={e.seq} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2 text-[13px]">
                      <Link to={`/cooks/${agent}`} className={`font-medium ${linkClass}`}>
                        {agent}
                      </Link>
                      <span className="text-muted-foreground">{started ? "started" : code ? `finished with exit code ${code}` : "finished"}</span>
                      {e.project && <PotLink pot={e.project} />}
                      <span className="ml-auto font-mono text-[10.5px] text-muted-foreground">{fmtAgo(e.at, now)}</span>
                    </li>
                  );
                })}
              </ul>
            )}
          </Section>
        </div>
      </PageShell>
    </>
  );
}
