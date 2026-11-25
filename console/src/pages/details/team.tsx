import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { Minus, Pencil, Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import type { MemberView, ProjectSummary } from "@corntchap/api";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, Group, GroupRow, PersonName, RowSkeleton } from "@/components/bits";
import { ConfirmDialog, DeleteDialog, RenameDialog } from "@/components/rename-delete";
import { SourceList } from "@/components/source-list";
import { AddMember } from "@/pages/admin";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import { useProcCall, useV1Call } from "@/lib/data";
import { usePotsSource, useTeamMembersSource } from "@/lib/sources";
import { useTeamByName } from "@/lib/people";
import { canManageTeam, useSession } from "@/lib/session";
import { Crumb, linkClass } from "./common";

const memberKey = (m: MemberView) => `${m.kind}:${m.name}`;
const potKey = (p: ProjectSummary) => p.key;

export function TeamDetailsPage() {
  const { name = "" } = useParams();
  const me = useSession();
  const found = useTeamByName(name);
  const team = found.item;
  const call = useV1Call();
  const proc = useProcCall();
  const navigate = useNavigate();
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<MemberView | null>(null);
  const [dialog, setDialog] = useState<"rename" | "delete" | null>(null);

  const members = useTeamMembersSource(team?.name ?? "", { enabled: !!team });
  const pots = usePotsSource({ team: team?.name ?? "", enabled: !!team });
  const canManage = !!team && canManageTeam(me, team.role);

  const crumb = <Crumb to="/teams" parent="Teams" here={name} />;

  if (found.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Team" title={name} className="pt-2">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (found.error || !team) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Team" title={name} className="pt-2">
          <EmptyState title={found.error ? "Could not reach the teams" : "No such team"} action={<Button asChild variant="outline"><Link to="/teams">Back to Teams</Link></Button>}>
            {found.error ? errorMessage(found.error) : `${name} is not a team you are in.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  const roleText = team.role ? (team.role === "member" ? "you are a chef" : `you are an ${team.role}`) : null;

  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Team"
        title={team.name}
        className="pt-2 sm:pt-3"
        sub={
          <>
            Teams decide who sees which pots. Their chefs can be people or cooks.
            <span className="mt-1 block font-mono text-[12px]">
              {team.members} {team.members === 1 ? "chef" : "chefs"}
              {roleText && <> · {roleText}</>}
            </span>
          </>
        }
        actions={
          <Button onClick={() => setAdding(true)}>
            <UserPlus /> {canManage ? "Add chef" : "Add my cook"}
          </Button>
        }
      >
        <div className="space-y-6 lg:grid lg:grid-cols-2 lg:items-start lg:gap-x-6 lg:space-y-0 lg:gap-y-6">
          <Group title="Chefs" aside={<span className="font-mono text-[11px] text-muted-foreground">{team.members}</span>}>
            <SourceList<MemberView>
              source={members}
              more="button"
              itemKey={memberKey}
              label="Chefs"
              empty={
                <>
                  <p className="font-medium text-foreground">Nobody is in this team yet</p>
                  <p>{canManage ? "Add a person or a cook." : "Add one of your cooks."}</p>
                </>
              }
              renderItem={(m) => (
                <div className="flex min-h-14 items-center gap-3 px-4 py-2">
                  <Avatar name={m.name} className="size-8 rounded-lg text-[14px]" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-[14px] leading-tight">
                      {m.kind === "user" ? (
                        <PersonName name={m.name} displayName={m.display_name} />
                      ) : (
                        <Link to={`/cooks/${m.name}`} className={`font-medium ${linkClass}`}>
                          {m.name}
                        </Link>
                      )}
                    </p>
                    <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                      {m.kind === "user" ? "person" : "cook"} · {m.role === "member" ? "chef" : m.role}
                      {m.fireAndForget && " · fire and forget"}
                    </p>
                  </div>
                  {(canManage || m.mine) && (
                    <Button type="button" size="icon" variant="ghost" aria-label={`Remove ${m.name} from ${team.name}`} onClick={() => setRemoving(m)}>
                      <Minus />
                    </Button>
                  )}
                </div>
              )}
            />
            <GroupRow icon={<UserPlus />} label={canManage ? "Add a chef" : "Add my cook"} onClick={() => setAdding(true)} />
          </Group>

          <div className="space-y-6">
            <Group title="Pots" hint="The pots this team owns: its chefs see them.">
              <SourceList<ProjectSummary>
                source={pots}
                itemKey={potKey}
                label="Pots"
                empty={
                  <>
                    <p className="font-medium text-foreground">No pot yet</p>
                    <p>An owner or a super-admin starts pots for a team.</p>
                  </>
                }
                renderItem={(p) => (
                  <GroupRow label={p.name} hint={<span className="font-mono">{p.key} · {p.open + p.claimed + p.blocked} not served</span>} to={`/pots/${p.key}/about`} />
                )}
              />
            </Group>

            {canManage && (
              <Group title="Danger zone" tone="danger">
                <GroupRow icon={<Pencil />} label="Rename" hint="Its pots and chefs stay as they are" onClick={() => setDialog("rename")} />
                <GroupRow
                  icon={<Trash2 />}
                  label="Delete"
                  hint={team.projects.length > 0 ? "It still has pots: delete or empty them first" : "Memberships go; people and cooks stay. This cannot be undone"}
                  destructive
                  onClick={() => setDialog("delete")}
                />
              </Group>
            )}
          </div>
        </div>
        <AddMember team={adding ? team.name : null} onClose={() => setAdding(false)} ownCooksOnly={!canManage} />
      </PageShell>

      <ConfirmDialog
        open={!!removing}
        onOpenChange={(o) => !o && setRemoving(null)}
        title={`Remove ${removing?.name ?? "them"} from ${team.name}?`}
        description="They stop seeing this team's pots. The person or cook itself stays."
        action="Remove"
        destructive
        onConfirm={async () => {
          await call("DELETE", `/teams/${team.name}/members/${removing!.name}`);
          toast.success(`${removing!.name} left ${team.name}`);
        }}
      />
      <RenameDialog
        open={dialog === "rename"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Rename ${team.name}`}
        description="Its pots and chefs stay as they are."
        label="New name"
        current={team.name}
        onSave={async (newName) => {
          await proc("teamRename", { name: team.name, newName });
          toast.success(`${team.name} is now ${newName.toLowerCase()}`);
          void navigate(`/teams/${newName.toLowerCase()}`, { replace: true });
        }}
      />
      <DeleteDialog
        open={dialog === "delete"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Delete ${team.name}?`}
        description={
          team.projects.length > 0
            ? `It still has pots (${team.projects.join(", ")}): delete or empty them first, the server refuses until then.`
            : "Its memberships go; the people and cooks stay. This cannot be undone."
        }
        confirmText={team.name}
        action="Delete the team"
        onConfirm={async () => {
          await proc("teamDelete", { name: team.name, confirm: team.name });
          toast.success(`${team.name} deleted`);
          void navigate("/teams", { replace: true });
        }}
      />
    </>
  );
}
