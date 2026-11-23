import { useState } from "react";
import { useNavigate } from "react-router";
import type { ProjectSummary } from "@corntchap/api";
import { Pencil, Settings2, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { DeleteDialog, RenameDialog } from "@/components/rename-delete";
import { useProcCall, useV1, type TeamRow } from "@/lib/data";
import { useSession } from "@/lib/session";

type PotRef = Pick<ProjectSummary, "key" | "name">;

/** Rename a pot's display name: the key never changes (it is part of every task id and MCP URL). */
export function PotRenameDialog({ pot, open, onOpenChange }: { pot: PotRef; open: boolean; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  return (
    <RenameDialog
      open={open}
      onOpenChange={onOpenChange}
      title={`Rename ${pot.key}`}
      description={
        <>
          Changes the pot's display name only. The key <b className="font-mono">{pot.key}</b> stays: it is part of every task id and of the cooks' MCP URLs.
        </>
      }
      label="Name"
      current={pot.name}
      mono={false}
      onSave={async (name) => {
        await call("potRename", { key: pot.key, name });
        toast.success(`${pot.key} is now called ${name}`);
      }}
    />
  );
}

/** Delete a pot and everything in it; the key has to be typed. */
export function PotDeleteDialog({
  pot,
  open,
  onOpenChange,
  onDeleted,
}: {
  pot: PotRef;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onDeleted?: () => void;
}) {
  const call = useProcCall();
  return (
    <DeleteDialog
      open={open}
      onOpenChange={onOpenChange}
      title={`Delete ${pot.name}?`}
      description={
        <>
          This deletes <b>{pot.key}</b> and everything in it for good: its tasks, checklists, comments, activity, pass, pantry, saved views, menus, stations and files (the stored
          files too). Cooks and people stay. This cannot be undone.
        </>
      }
      confirmText={pot.key}
      action="Delete the pot"
      onConfirm={async () => {
        const out = await call("potDelete", { key: pot.key, confirm: pot.key });
        toast.success(`${pot.key} deleted`, { description: `${out.tasks} ${out.tasks === 1 ? "task" : "tasks"} went with it.` });
        onDeleted?.();
      }}
    />
  );
}

/** The pot's options menu (rename, delete) for the people who may use it: its team's owners and admins. */
export function PotOptions({ project }: { project: ProjectSummary }) {
  const me = useSession();
  const teams = useV1<TeamRow[]>("/teams");
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<"rename" | "delete" | null>(null);
  const owner = teams.data?.find((t) => t.name === project.team)?.role === "owner";
  if (!me.is_admin && !owner) return null;
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={`Options for ${project.name}`}
          className="inline-flex size-9 shrink-0 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <Settings2 className="size-4" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => setDialog("rename")}>
            <Pencil /> Rename
          </DropdownMenuItem>
          <DropdownMenuItem variant="destructive" onSelect={() => setDialog("delete")}>
            <Trash2 /> Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <PotRenameDialog pot={project} open={dialog === "rename"} onOpenChange={(o) => !o && setDialog(null)} />
      <PotDeleteDialog pot={project} open={dialog === "delete"} onOpenChange={(o) => !o && setDialog(null)} onDeleted={() => void navigate("/", { replace: true })} />
    </>
  );
}
