import { useState } from "react";
import { Link, useParams } from "react-router";
import { Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, PersonName, RowSkeleton } from "@/components/bits";
import { AddMember } from "@/pages/admin";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import { useProjects, useTeamChefs, useV1, useV1Call, type TeamRow } from "@/lib/data";
import { useSession } from "@/lib/session";
import { Crumb, PotLink, Quiet, Section, linkClass } from "./common";

export function TeamDetailsPage() {
  const { name = "" } = useParams();
  const me = useSession();
  const teams = useV1<TeamRow[]>("/teams");
  const projects = useProjects();
  const call = useV1Call();
  const [adding, setAdding] = useState(false);

  const team = (teams.data ?? []).find((t) => t.name.toLowerCase() === name.toLowerCase());
  const members = useTeamChefs(team?.name);
  const canManage = !!team && (me.is_admin || team.role === "owner");
  const pots = (projects.data ?? []).filter((p) => p.team?.toLowerCase() === name.toLowerCase());

  const crumb = <Crumb to="/teams" parent="Teams" here={name} />;

  if (teams.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Team" title={name} className="pt-3">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (teams.error || !team) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Team" title={name} className="pt-3">
          <EmptyState title={teams.error ? "Could not reach the teams" : "No such team"} action={<Button asChild variant="outline"><Link to="/teams">Back to Teams</Link></Button>}>
            {teams.error ? errorMessage(teams.error) : `${name} is not a team you are in.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Team"
        title={team.name}
        className="pt-3"
        sub="Teams decide who sees which pots. Their chefs can be people or cooks."
        actions={
          canManage ? (
            <Button onClick={() => setAdding(true)}>
              <UserPlus /> Add chef
            </Button>
          ) : undefined
        }
      >
        <div className="space-y-4">
          <Section title="Chefs" hint={`${team.members} ${team.members === 1 ? "chef" : "chefs"}${team.role ? ` · you are ${team.role === "member" ? "a chef" : `an ${team.role}`}` : ""}`}>
            {members.isLoading ? (
              <RowSkeleton rows={2} />
            ) : (members.data ?? []).length === 0 ? (
              <Quiet>Nobody is in this team yet.</Quiet>
            ) : (
              <ul className="divide-y divide-border">
                {members.data!.map((m) => (
                  <li key={m.name} className="flex items-center gap-3 py-2.5">
                    <Avatar name={m.name} className="size-7 text-[13px]" />
                    {m.kind === "user" ? (
                      <PersonName name={m.name} displayName={m.display_name} />
                    ) : (
                      <Link to={`/cooks/${m.name}`} className={`font-medium ${linkClass}`}>
                        {m.name}
                      </Link>
                    )}
                    <span className="rounded-full bg-muted px-2 py-px text-[11px] text-muted-foreground">{m.kind === "user" ? "person" : "cook"}</span>
                    <span className="font-mono text-[11px] text-muted-foreground">{m.role === "member" ? "chef" : m.role}</span>
                    {canManage && (
                      <Button
                        size="icon-xs"
                        variant="ghost"
                        className="ml-auto"
                        aria-label={`Remove ${m.name} from ${team.name}`}
                        onClick={async () => {
                          try {
                            await call("DELETE", `/teams/${team.name}/members/${m.name}`);
                            toast.success(`${m.name} left ${team.name}`);
                          } catch (e) {
                            toast.error("Could not remove them", { description: errorMessage(e) });
                          }
                        }}
                      >
                        <Trash2 />
                      </Button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </Section>

          <Section title="Pots" hint="The pots this team owns: its chefs see them.">
            {projects.isLoading ? (
              <RowSkeleton rows={1} />
            ) : pots.length === 0 ? (
              <Quiet>This team has no pot yet.</Quiet>
            ) : (
              <ul className="divide-y divide-border">
                {pots.map((p) => (
                  <li key={p.key} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                    <PotLink pot={p.key} />
                    <span className="text-[13.5px]">{p.name}</span>
                    <span className="ml-auto font-mono text-[11.5px] text-muted-foreground">{p.open + p.claimed + p.blocked} not served</span>
                    <Link to={`/pots/${p.key}`} className={`text-[12.5px] ${linkClass}`}>
                      Board
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
        <AddMember team={adding ? team.name : null} onClose={() => setAdding(false)} />
      </PageShell>
    </>
  );
}
