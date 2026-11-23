import { useMemo } from "react";
import { Link, useParams } from "react-router";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, Group, GroupRow, RowSkeleton, StatePill, cookState } from "@/components/bits";
import { CopyLine } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { FireBadge, FireManage, fireState } from "@/pages/admin";
import { errorMessage } from "@/lib/api";
import { useAgents, useV1, type AgentRow } from "@/lib/data";
import { fmtAgo, useNow } from "@/lib/format";
import { useLive } from "@/lib/live";
import { Crumb, linkClass, useWakeTargetsOf } from "./common";

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
        <PageShell eyebrow="Fire" title={name} className="pt-2">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (relays.error || !fire) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Fire" title={name} className="pt-2">
          <EmptyState title={relays.error ? "Could not reach the fires" : "No such fire"} action={<Button asChild variant="outline"><Link to="/fires">Back to Fires</Link></Button>}>
            {relays.error ? errorMessage(relays.error) : `${name} is not a fire you can see.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  const { lit, label } = fireState(fire, now);
  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Fire"
        title={fire.name}
        className="pt-2 sm:pt-3"
        sub={
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
            <FireBadge lit={lit} label={label} />
            <span>A relay on a machine of its owner: it listens for work and wakes that owner's local cooks.</span>
          </span>
        }
      >
        <div className="space-y-6 lg:grid lg:grid-cols-2 lg:items-start lg:gap-x-6 lg:space-y-0 lg:gap-y-6">
          <div className="space-y-6">
            <Group title="Overview">
              <GroupRow
                label="State"
                hint={lit ? "Heard from in the last five minutes" : label === "put out" ? "Put out: it cannot listen" : "Not heard from lately"}
                trailing={<FireBadge lit={lit} label={label} />}
              />
              <GroupRow label="Last seen" value={fmtAgo(fire.last_seen_at, now)} />
              <GroupRow label="Owner" value={fire.owner ?? "none"} />
            </Group>

            <Group title="Enroll on a machine" hint="Run it where the cooks live, then approve the device in the browser.">
              <div className="p-3">
                <CopyLine text={`corntchap-cli fire enroll ${fire.name}`} />
              </div>
            </Group>

            <Group title="Cooks it wakes" hint="The owner's cooks that have a relay wake target.">
              {agents.isLoading || wake.isLoading ? (
                <div className="p-3">
                  <RowSkeleton rows={2} />
                </div>
              ) : wakes.length === 0 ? (
                <GroupRow label="None yet" hint={`No cook of ${fire.owner ?? "its owner"} has a relay wake target. Add one on the cook, or run corntchap-cli cook setup.`} />
              ) : (
                wakes.map((a) => {
                  const cs = cookState(a, now);
                  return (
                    <Link
                      key={a.name}
                      to={`/cooks/${a.name}`}
                      className="flex min-h-14 items-center gap-3 px-4 py-2.5 outline-none transition-colors hover:bg-muted/50 focus-visible:bg-muted/60 active:bg-muted"
                    >
                      <Avatar name={a.name} className="size-8 rounded-lg text-[14px]" />
                      <span className="min-w-0 flex-1">
                        <span className="block text-[14px] font-medium leading-tight">{a.name}</span>
                        {(a.teams ?? []).length > 0 && <span className="block truncate font-mono text-[11px] text-muted-foreground">{(a.teams ?? []).join(" · ")}</span>}
                      </span>
                      <StatePill state={cs} />
                    </Link>
                  );
                })
              )}
            </Group>
          </div>

          <div className="space-y-6">
            <Group title="Recent runs" hint="Each time the relay started one of these cooks and when it finished.">
              {runs.length === 0 && <GroupRow label="No run reported" hint="Since this page's feed began." />}
              {runs.map((e) => {
                const agent = e.payload.agent ?? e.actor ?? "";
                const code = (e.payload as { exit_code?: number | null }).exit_code;
                const started = e.kind === "run.started";
                return (
                  <GroupRow
                    key={e.seq}
                    label={
                      <Link to={`/cooks/${agent}`} className={linkClass}>
                        {agent}
                      </Link>
                    }
                    hint={
                      <>
                        {started ? "started" : code ? `finished with exit code ${code}` : "finished"}
                        {e.project && <span className="font-mono"> · {e.project}</span>}
                      </>
                    }
                    value={<span className="font-mono text-[11px]">{fmtAgo(e.at, now)}</span>}
                  />
                );
              })}
            </Group>
            <FireManage fire={fire} />
          </div>
        </div>
      </PageShell>
    </>
  );
}
