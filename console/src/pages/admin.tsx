import { useMemo, useState, useEffect, type FormEvent, type ReactNode } from "react";
import { Link, Navigate, useNavigate } from "react-router";
import { Check, ChevronRight, Flame, Loader2, Pencil, Plus, Power, RotateCw, Search, ShieldCheck, Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { DataTable, PageShell, TD, TH } from "@/components/page";
import {
  Avatar,
  EmptyState,
  FormSheetBody,
  FormSheetFoot,
  FormSheetHead,
  Group,
  GroupRow,
  PersonName,
  RowSkeleton,
  cookState,
  formSheetClass,
} from "@/components/bits";
import { CopyLine, TokenDialog } from "@/components/secret-dialog";
import { ConfirmDialog, DeleteDialog, RenameDialog } from "@/components/rename-delete";
import { PotDeleteDialog, PotRenameDialog } from "@/components/pot-options";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useAgents, useProcCall, useProjects, useTeamChefs, useV1, useV1Call, type AgentRow, type TeamRow, type UserRow } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useSession } from "@/lib/session";

export function AdminOnly({ children }: { children: ReactNode }) {
  const me = useSession();
  if (!me.is_admin) return <Navigate to="/" replace />;
  return <>{children}</>;
}

/** A form in a dialog: a bottom sheet on phones (the body scrolls, the button stays put under the keyboard). */
function FormDialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  onSubmit,
  busy,
  submitLabel,
  disabled,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  description: string;
  children: ReactNode;
  onSubmit: () => void | Promise<void>;
  busy: boolean;
  submitLabel: string;
  disabled?: boolean;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className={cn(formSheetClass, "sm:max-w-md")}>
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(e: FormEvent) => {
            e.preventDefault();
            void onSubmit();
          }}
        >
          <FormSheetHead>
            <DialogTitle className="text-[22px] leading-tight">{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </FormSheetHead>
          <FormSheetBody className="pb-3">{children}</FormSheetBody>
          <FormSheetFoot>
            <Button type="submit" disabled={busy || disabled} className="max-sm:h-11">
              {busy && <Loader2 className="animate-spin" />} {submitLabel}
            </Button>
            <Button type="button" variant="ghost" className="max-sm:h-11" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
          </FormSheetFoot>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ------------------------------------------------------------------ Fires (relays)

/** Same rule as the sidebar's fire card: awake within five minutes and not put out is lit. */
export function fireState(r: AgentRow, now: number) {
  const st = cookState({ ...r, busy: 0 }, now);
  const lit = st === "awake";
  return { lit, label: lit ? "lit" : r.disabled ? "put out" : "banked" };
}

/** Lit (saffron, pulsing) or not (quiet): the one thing a fire's page has to say. */
export function FireBadge({ lit, label, className }: { lit: boolean; label: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11.5px] font-semibold uppercase tracking-wider",
        lit ? "border-corn/60 bg-corn/15 text-corn-text" : label === "put out" ? "border-destructive/40 text-destructive" : "border-border bg-muted/60 text-muted-foreground",
        className,
      )}
    >
      <span aria-hidden="true" className={cn("size-2 rounded-full", lit ? "live-dot bg-corn" : label === "put out" ? "bg-destructive" : "bg-muted-foreground/50")} />
      {label}
    </span>
  );
}

export function FireTile({ lit, className }: { lit: boolean; className?: string }) {
  return (
    <span className={cn("grid size-11 shrink-0 place-items-center rounded-xl", lit ? "bg-corn/20" : "bg-muted", className)}>
      <Flame className={cn("size-5", lit ? "text-corn-text" : "text-muted-foreground")} aria-hidden="true" />
    </span>
  );
}

/** Rotate, put out, rename and delete a fire: shared by the fire's own page. */
export function FireManage({ fire }: { fire: AgentRow }) {
  const call = useV1Call();
  const proc = useProcCall();
  const navigate = useNavigate();
  const [dlg, setDlg] = useState<"rotate" | "putout" | "rename" | "delete" | null>(null);
  const [token, setToken] = useState<{ token: string; title: string } | null>(null);

  async function setEnabled(enable: boolean) {
    try {
      await call("POST", `/agents/${fire.name}/${enable ? "enable" : "disable"}`);
      toast.success(enable ? `${fire.name} lit again` : `${fire.name} put out`);
    } catch (e) {
      toast.error("Could not change that", { description: errorMessage(e) });
    }
  }

  return (
    <>
      <Group title="Token" hint="The relay on the machine needs the new token to listen again.">
        <GroupRow icon={<RotateCw />} label="Rotate token" hint="Make a new token; the old one stops working at once" onClick={() => setDlg("rotate")} />
      </Group>
      <Group title="Danger zone" tone="danger">
        {fire.disabled ? (
          <GroupRow icon={<Flame />} label="Light again" hint="Let the relay listen for work again" onClick={() => void setEnabled(true)} />
        ) : (
          <GroupRow icon={<Power />} label="Put out" hint="The relay stops listening until you light it again" destructive onClick={() => setDlg("putout")} />
        )}
        <GroupRow icon={<Pencil />} label="Rename" hint="The relay keeps working: it is known by its token" onClick={() => setDlg("rename")} />
        <GroupRow icon={<Trash2 />} label="Delete" hint="Its tokens stop working. This cannot be undone" destructive onClick={() => setDlg("delete")} />
      </Group>

      <ConfirmDialog
        open={dlg === "rotate"}
        onOpenChange={(o) => !o && setDlg(null)}
        title={`Rotate ${fire.name}'s token?`}
        description="The running relay stops being able to listen until it gets the new token."
        action="Rotate it"
        onConfirm={async () => {
          try {
            const r = await call<{ token: string }>("POST", `/relays/${fire.name}/rotate`);
            setToken({ token: r.token, title: `New token for ${fire.name}` });
          } catch (e) {
            toast.error("Could not rotate the token", { description: errorMessage(e) });
          }
        }}
      />
      <ConfirmDialog
        open={dlg === "putout"}
        onOpenChange={(o) => !o && setDlg(null)}
        title={`Put out ${fire.name}?`}
        description="The relay can no longer listen, so local cooks are not woken by it. You can light it again."
        action="Put it out"
        destructive
        onConfirm={() => setEnabled(false)}
      />
      <TokenDialog
        open={!!token}
        onOpenChange={(o) => !o && setToken(null)}
        title={token?.title ?? ""}
        name={fire.name}
        token={token?.token ?? null}
        kind={null}
        withSnippets={false}
        hint="Store it as the relay credential: corntchap-cli fire enroll does this for you, or set CORNTCHAP_TOKEN for the relay process."
      />
      <RenameDialog
        open={dlg === "rename"}
        onOpenChange={(o) => !o && setDlg(null)}
        title={`Rename ${fire.name}`}
        description="The relay keeps working: it is known by its token, not by its name."
        label="New name"
        current={fire.name}
        onSave={async (newName) => {
          await proc("fireRename", { name: fire.name, newName });
          toast.success(`${fire.name} is now ${newName.toLowerCase()}`);
          void navigate(`/fires/${newName.toLowerCase()}`, { replace: true });
        }}
      />
      <DeleteDialog
        open={dlg === "delete"}
        onOpenChange={(o) => !o && setDlg(null)}
        title={`Delete ${fire.name}?`}
        description="Its tokens stop working at once, so the relay on that machine can no longer listen. Enroll a new fire to get going again. This cannot be undone."
        action="Delete the fire"
        onConfirm={async () => {
          await proc("fireDelete", { name: fire.name });
          toast.success(`${fire.name} deleted`);
          void navigate("/fires", { replace: true });
        }}
      />
    </>
  );
}

export function FiresPage() {
  const relays = useV1<AgentRow[]>("/relays");
  const call = useV1Call();
  const now = useNow();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [token, setToken] = useState<{ name: string; token: string; title: string } | null>(null);

  async function create() {
    setBusy(true);
    try {
      const r = await call<{ name: string; token: string }>("POST", "/relays", { name });
      setCreating(false);
      setName("");
      setToken({ name: r.name, token: r.token, title: `${r.name} is lit` });
    } catch (e) {
      toast.error("Could not light that fire", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  const rows = relays.data ?? [];
  const litCount = rows.filter((r) => fireState(r, now).lit).length;

  return (
    <PageShell
      eyebrow="Kitchen"
      title="Fires"
      sub={
        <>
          A fire is a relay running on your laptop: it listens for work and wakes your local cooks. Cloud cooks are woken by a routine, a GitHub comment or a webhook instead.
          {rows.length > 0 && (
            <span className="mt-1 block font-mono text-[12px]">
              {litCount} of {rows.length} lit
            </span>
          )}
        </>
      }
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> Light a fire
        </Button>
      }
    >
      {relays.isLoading ? (
        <RowSkeleton rows={2} />
      ) : rows.length === 0 ? (
        <EmptyState title="No fire is lit" action={<Button onClick={() => setCreating(true)}>Light the first fire</Button>}>
          Enroll a relay on the machine where your cooks live: run <code className="font-mono">corntchap-cli fire enroll relay-laptop</code> there and approve it in the browser, or make one here.
        </EmptyState>
      ) : (
        <ul className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
          {[...rows]
            .sort((a, b) => Number(fireState(b, now).lit) - Number(fireState(a, now).lit) || a.name.localeCompare(b.name))
            .map((r) => {
              const { lit, label } = fireState(r, now);
              return (
                <li key={r.name} className={cn("overflow-hidden rounded-[14px] border bg-card", lit ? "border-corn/50" : "border-border")}>
                  <Link
                    to={`/fires/${r.name}`}
                    aria-label={`Fire ${r.name}, ${label}`}
                    className="flex min-h-[4.5rem] items-center gap-3 p-4 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted"
                  >
                    <FireTile lit={lit} />
                    <div className="min-w-0 flex-1">
                      <h2 className="truncate font-display text-[18px] leading-tight">{r.name}</h2>
                      <p className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
                        seen {fmtAgo(r.last_seen_at, now)}
                        {r.owner && <> · {r.owner}</>}
                      </p>
                    </div>
                    <FireBadge lit={lit} label={label} />
                    <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
                  </Link>
                  <div className="border-t border-border bg-muted/30 p-2.5">
                    <CopyLine text={`corntchap-cli fire enroll ${r.name}`} />
                  </div>
                </li>
              );
            })}
        </ul>
      )}

      <FormDialog
        open={creating}
        onOpenChange={setCreating}
        title="Light a fire"
        description="Creates a relay and shows its token once. Prefer the device flow on the laptop when you can."
        onSubmit={create}
        busy={busy}
        submitLabel="Light it"
        disabled={!name}
      >
        <div className="space-y-1.5">
          <Label htmlFor="fire-name">Name</Label>
          <Input
            id="fire-name"
            required
            autoFocus
            autoCapitalize="none"
            spellCheck={false}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="relay-laptop"
            pattern="[A-Za-z0-9][A-Za-z0-9._-]*"
            className="h-11 font-mono"
          />
        </div>
      </FormDialog>
      <TokenDialog
        open={!!token}
        onOpenChange={(o) => !o && setToken(null)}
        title={token?.title ?? ""}
        name={token?.name ?? ""}
        token={token?.token ?? null}
        kind={null}
        withSnippets={false}
        hint="Store it as the relay credential: corntchap-cli fire enroll does this for you, or set CORNTCHAP_TOKEN for the relay process."
      />
    </PageShell>
  );
}

// ------------------------------------------------------------------ Pots (admin)

const leaseText = (s: number) => (s >= 120 ? `${Math.round(s / 60)} min` : `${s} s`);

export function PotsAdminPage() {
  const projects = useProjects();
  const teams = useV1<TeamRow[]>("/teams");
  const call = useV1Call();
  const [creating, setCreating] = useState(false);
  const [team, setTeam] = useState("");
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [repo, setRepo] = useState("");
  const [ttl, setTtl] = useState("900");
  const [busy, setBusy] = useState(false);
  const [potDialog, setPotDialog] = useState<{ kind: "rename" | "delete"; key: string } | null>(null);
  const dialogPot = projects.data?.find((p) => p.key === potDialog?.key);

  async function create() {
    setBusy(true);
    try {
      await call("POST", "/projects", {
        team: team || teams.data?.[0]?.name,
        key,
        name,
        repo_url: repo || undefined,
        lease_ttl_s: Number(ttl) || undefined,
      });
      toast.success(`${key.toUpperCase()} is on the stove`);
      setCreating(false);
      setKey("");
      setName("");
      setRepo("");
    } catch (e) {
      toast.error("Could not start that pot", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  const rows = projects.data ?? [];

  return (
    <PageShell
      eyebrow="Admin"
      title="Pots"
      sub="Pots are projects: a key like PAY, a team that may see them and a default lease length for scooped tasks."
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> New pot
        </Button>
      }
    >
      {projects.isLoading ? (
        <RowSkeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No pots yet" action={<Button onClick={() => setCreating(true)}>Start the first pot</Button>}>
          Make a team first if you have none, then start a pot for it.
        </EmptyState>
      ) : (
        <>
          {/* phones: two-line rows, a tap opens the pot (rename and delete live there) */}
          <ul className="divide-y divide-border overflow-hidden rounded-[14px] border border-border bg-card md:hidden">
            {rows.map((p) => (
              <li key={p.key}>
                <Link to={`/pots/${p.key}/about`} className="flex min-h-16 items-center gap-3 px-4 py-3 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted">
                  <span className="rounded-md bg-muted px-2 py-1 font-mono text-[12.5px] font-bold">{p.key}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[14.5px] font-medium leading-tight">{p.name}</span>
                    <span className="mt-0.5 block truncate font-mono text-[11.5px] text-muted-foreground">
                      {p.team ?? "no team"} · lease {leaseText(p.lease_ttl_s)}
                    </span>
                  </span>
                  <span className="text-right font-mono text-[12.5px] leading-tight">
                    <b>{p.open + p.claimed + p.blocked}</b>
                    <span className="block text-[10px] uppercase tracking-wider text-muted-foreground">not served</span>
                  </span>
                  <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
                </Link>
              </li>
            ))}
          </ul>
          <DataTable className="max-md:hidden">
            <thead>
              <tr>
                <TH>Key</TH>
                <TH>Name</TH>
                <TH>Team</TH>
                <TH>Lease</TH>
                <TH className="text-right">Not served</TH>
                <TH className="text-right">
                  <span className="sr-only">Actions</span>
                </TH>
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => (
                <tr key={p.key}>
                  <TD className="font-mono text-[12.5px] font-bold">
                    <Link to={`/pots/${p.key}/about`} className="underline-offset-2 hover:underline focus-visible:underline">
                      {p.key}
                    </Link>
                  </TD>
                  <TD>
                    {p.name}
                    {p.repo_url && <span className="ml-2 font-mono text-[11px] text-muted-foreground">{p.repo_url}</span>}
                  </TD>
                  <TD>{p.team ?? "-"}</TD>
                  <TD className="font-mono text-[12px]">{Math.round(p.lease_ttl_s / 60)} min</TD>
                  <TD className="text-right font-mono text-[12.5px]">{p.open + p.claimed + p.blocked}</TD>
                  <TD className="whitespace-nowrap text-right">
                    <Button size="icon-sm" variant="ghost" aria-label={`Rename ${p.key}`} onClick={() => setPotDialog({ kind: "rename", key: p.key })}>
                      <Pencil />
                    </Button>
                    <Button size="icon-sm" variant="ghost" className="text-destructive hover:text-destructive" aria-label={`Delete ${p.key}`} onClick={() => setPotDialog({ kind: "delete", key: p.key })}>
                      <Trash2 />
                    </Button>
                  </TD>
                </tr>
              ))}
            </tbody>
          </DataTable>
        </>
      )}
      {dialogPot && <PotRenameDialog pot={dialogPot} open={potDialog?.kind === "rename"} onOpenChange={(o) => !o && setPotDialog(null)} />}
      {dialogPot && <PotDeleteDialog pot={dialogPot} open={potDialog?.kind === "delete"} onOpenChange={(o) => !o && setPotDialog(null)} />}
      <FormDialog
        open={creating}
        onOpenChange={setCreating}
        title="New pot"
        description="A project that tasks live in. Its key prefixes every task id."
        onSubmit={create}
        busy={busy}
        submitLabel="Start the pot"
        disabled={!key || !name || !(team || teams.data?.[0])}
      >
        <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="pot-key">Key</Label>
            <Input id="pot-key" required autoFocus autoCapitalize="characters" value={key} onChange={(e) => setKey(e.target.value.toUpperCase())} placeholder="PAY" className="h-11 font-mono" pattern="[A-Za-z][A-Za-z0-9]{0,9}" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="pot-name">Name</Label>
            <Input id="pot-name" required value={name} onChange={(e) => setName(e.target.value)} placeholder="Payments" className="h-11" />
          </div>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="pot-team">Team</Label>
          <Select value={team || teams.data?.[0]?.name || ""} onValueChange={setTeam}>
            <SelectTrigger id="pot-team" className="h-11 w-full">
              <SelectValue placeholder="Make a team first" />
            </SelectTrigger>
            <SelectContent>
              {(teams.data ?? []).map((t) => (
                <SelectItem key={t.name} value={t.name}>
                  {t.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="pot-repo">Repository (optional)</Label>
          <Input id="pot-repo" type="url" value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="https://github.com/acme/payments" className="h-11" />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="pot-ttl">Lease length (seconds)</Label>
          <Input id="pot-ttl" type="number" inputMode="numeric" min={30} max={86400} value={ttl} onChange={(e) => setTtl(e.target.value)} className="h-11 font-mono" />
          <p className="text-[12px] text-muted-foreground">How long a scooped task stays held without a heartbeat. 900 is 15 minutes.</p>
        </div>
      </FormDialog>
    </PageShell>
  );
}

// ------------------------------------------------------------------ Teams (admin)

export function TeamsAdminPage() {
  const me = useSession();
  const teams = useV1<TeamRow[]>("/teams");
  const call = useV1Call();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);

  async function create() {
    setBusy(true);
    try {
      await call("POST", "/teams", { name });
      toast.success(`Team ${name} is set`);
      setCreating(false);
      setName("");
    } catch (e) {
      toast.error("Could not make that team", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageShell
      eyebrow={me.is_admin ? "Admin" : "Kitchen"}
      title="Teams"
      sub="Teams decide who sees which pots. Their chefs can be people or cooks."
      actions={
        me.is_admin ? (
          <Button onClick={() => setCreating(true)}>
            <Plus /> New team
          </Button>
        ) : undefined
      }
    >
      {teams.isLoading ? (
        <RowSkeleton rows={3} />
      ) : (teams.data ?? []).length === 0 ? (
        me.is_admin ? (
          <EmptyState title="No teams yet" action={<Button onClick={() => setCreating(true)}>Make the first team</Button>}>
            A team owns pots and has chefs. Start with the people who cook together.
          </EmptyState>
        ) : (
          <EmptyState title="No teams">You are in no team yet. An admin creates teams and adds you.</EmptyState>
        )
      ) : (
        <ul className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
          {teams.data!.map((t) => (
            <TeamCard key={t.name} team={t} canManage={me.is_admin || t.role === "owner"} onAdd={() => setAdding(t.name)} />
          ))}
        </ul>
      )}
      <FormDialog open={creating} onOpenChange={setCreating} title="New team" description="You become its owner." onSubmit={create} busy={busy} submitLabel="Create team" disabled={!name}>
        <div className="space-y-1.5">
          <Label htmlFor="team-name">Name</Label>
          <Input id="team-name" required autoFocus autoCapitalize="none" value={name} onChange={(e) => setName(e.target.value)} placeholder="acme" className="h-11" />
        </div>
      </FormDialog>
      <AddMember team={adding} onClose={() => setAdding(null)} />
    </PageShell>
  );
}

function TeamCard({ team, canManage, onAdd }: { team: TeamRow; canManage: boolean; onAdd: () => void }) {
  const members = useTeamChefs(team.name);
  const list = members.data ?? [];
  return (
    <li className="overflow-hidden rounded-[14px] border border-border bg-card">
      <Link
        to={`/teams/${team.name}`}
        aria-label={`Team ${team.name}`}
        className="flex min-h-[4.5rem] items-center gap-3 p-4 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted"
      >
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-display text-[18px] leading-tight">{team.name}</h2>
          <p className="mt-0.5 truncate font-mono text-[11.5px] text-muted-foreground">
            {team.members} {team.members === 1 ? "chef" : "chefs"} · pots {team.projects.length ? team.projects.join(", ") : "none"}
          </p>
        </div>
        <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
      </Link>
      <div className="flex min-h-14 items-center justify-between gap-3 border-t border-border px-4 py-2.5">
        <div className="flex min-w-0 items-center" aria-label={`${team.members} members`}>
          {list.slice(0, 6).map((m) => (
            <Avatar key={m.name} name={m.name} className="-ml-1.5 size-7 rounded-full text-[12px] ring-2 ring-card first:ml-0" />
          ))}
          {list.length > 6 && <span className="ml-1.5 font-mono text-[11px] text-muted-foreground">+{list.length - 6}</span>}
          {members.isLoading && <span className="text-[12px] text-muted-foreground">Loading…</span>}
        </div>
        {canManage && (
          <Button variant="outline" size="sm" onClick={onAdd}>
            <UserPlus /> Add chef
          </Button>
        )}
      </div>
    </li>
  );
}

interface Candidate {
  /** What the API takes as `principal`: a person's handle or a cook's name. */
  principal: string;
  label: string;
  display?: string | null;
  kind: "user" | "cook";
}

export function AddMember({ team, onClose }: { team: string | null; onClose: () => void }) {
  const call = useV1Call();
  const me = useSession();
  const open = !!team;
  // /users is admin-only: an owner who is no admin picks from their cooks or types a handle
  const users = useV1<UserRow[]>("/users", { enabled: open && me.is_admin });
  const [handle, setHandle] = useState("");
  const cooks = useAgents();
  const members = useTeamChefs(team, { enabled: open });
  const [who, setWho] = useState<Candidate | null>(null);
  const [query, setQuery] = useState("");
  const [role, setRole] = useState("member");
  const [busy, setBusy] = useState(false);

  const candidates = useMemo(() => {
    const taken = new Set((members.data ?? []).map((m) => m.name));
    const people: Candidate[] = (users.data ?? [])
      .filter((u) => !u.disabled && !taken.has(u.name))
      .map((u) => ({ principal: u.name, label: u.name, display: u.display_name, kind: "user" as const }));
    const cookList: Candidate[] = (cooks.data ?? [])
      .filter((c) => !c.disabled && !taken.has(c.name))
      .map((c) => ({ principal: c.name, label: c.name, kind: "cook" as const }));
    return [...people, ...cookList];
  }, [users.data, cooks.data, members.data]);
  const q = query.trim().toLowerCase();
  const shown = candidates.filter((c) => !q || `${c.label} ${c.display ?? ""} ${c.kind}`.toLowerCase().includes(q));
  const loading = (me.is_admin && users.isLoading) || cooks.isLoading || members.isLoading;

  useEffect(() => {
    if (!open) {
      setWho(null);
      setRole("member");
      setQuery("");
      setHandle("");
    }
  }, [open]);
  const principal = who?.principal ?? (me.is_admin ? "" : handle.trim().replace(/^@/, ""));

  return (
    <FormDialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={`Add to ${team}`}
      description={me.is_admin ? "Pick a person or a cook that is not on this team yet." : "Pick one of your cooks, or type a person's handle."}
      busy={busy}
      submitLabel="Add"
      disabled={!principal}
      onSubmit={async () => {
        if (!principal) return;
        setBusy(true);
        try {
          await call("POST", `/teams/${team}/members`, { principal, role });
          toast.success(`${principal} joined ${team}`);
          onClose();
        } catch (e) {
          toast.error("Could not add them", { description: errorMessage(e) });
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="space-y-1.5">
        <Label htmlFor="mem-search">Chef</Label>
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            id="mem-search"
            type="search"
            autoComplete="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={me.is_admin ? "Search people and cooks" : "Search your cooks"}
            className="h-11 pl-9"
          />
        </div>
        <div role="radiogroup" aria-label="Chef" className="scrollbar-thin max-h-56 divide-y divide-border overflow-y-auto rounded-xl border border-border">
          {loading && <p className="px-3.5 py-3 text-[13px] text-muted-foreground">Loading…</p>}
          {!loading && shown.length === 0 && <p className="px-3.5 py-3 text-[13px] text-muted-foreground">Nobody left to add.</p>}
          {shown.map((c) => {
            const on = who?.kind === c.kind && who.principal === c.principal;
            return (
              <button
                key={`${c.kind}:${c.principal}`}
                type="button"
                role="radio"
                aria-checked={on}
                onClick={() => setWho(on ? null : c)}
                className={cn("flex min-h-12 w-full items-center gap-2.5 px-3.5 py-2 text-left transition-colors", on ? "bg-primary/5" : "hover:bg-muted/50")}
              >
                <Avatar name={c.label} className="size-7 text-[12px]" />
                {c.kind === "user" ? <PersonName name={c.label} displayName={c.display} className="min-w-0 flex-1 truncate" /> : <span className="min-w-0 flex-1 truncate font-medium">{c.label}</span>}
                <span className="rounded-full bg-muted px-2 py-px text-[11px] text-muted-foreground">{c.kind === "cook" ? "cook" : "person"}</span>
                {on && <Check className="size-4 text-primary" aria-hidden="true" />}
              </button>
            );
          })}
        </div>
      </div>
      {!me.is_admin && (
        <div className="space-y-1.5">
          <Label htmlFor="mem-handle">Or a person's handle</Label>
          <Input id="mem-handle" value={handle} disabled={!!who} onChange={(e) => setHandle(e.target.value)} placeholder="ada" autoComplete="off" autoCapitalize="none" className="h-11 font-mono" />
        </div>
      )}
      <div className="space-y-1.5">
        <Label>Role</Label>
        <div role="radiogroup" aria-label="Role" className="grid grid-cols-2 gap-2">
          {[
            { id: "member", label: "Chef" },
            { id: "owner", label: "Owner" },
          ].map((r) => (
            <button
              key={r.id}
              type="button"
              role="radio"
              aria-checked={role === r.id}
              onClick={() => setRole(r.id)}
              className={cn("h-11 rounded-xl border text-[13.5px] font-medium transition-colors", role === r.id ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50")}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>
    </FormDialog>
  );
}

// ------------------------------------------------------------------ Users (admin)

function RoleBadge({ u }: { u: UserRow }) {
  return u.is_admin ? (
    <span className="inline-flex items-center gap-1 rounded-full bg-primary/15 px-2 py-0.5 text-[11.5px] font-medium text-primary">
      <ShieldCheck className="size-3" aria-hidden="true" /> admin
    </span>
  ) : (
    <span className="text-[12.5px] text-muted-foreground">chef</span>
  );
}

const signIn = (u: UserRow) => (u.disabled ? "disabled" : u.linked ? "signed in" : "local (no sign-in)");

export function UsersAdminPage() {
  const users = useV1<UserRow[]>("/users");
  const rows = users.data ?? [];

  return (
    <PageShell
      eyebrow="Admin"
      title="Users"
      sub="People who have signed in through your identity provider. Each gets a kitchen handle at their first sign-in and can change it from the account menu."
    >
      {users.isLoading ? (
        <RowSkeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No users yet">People show up here after their first sign-in.</EmptyState>
      ) : (
        <>
          <ul className="divide-y divide-border overflow-hidden rounded-[14px] border border-border bg-card md:hidden">
            {rows.map((u) => (
              <li key={u.name} className="flex min-h-16 items-center gap-3 px-4 py-3">
                <Avatar name={u.name} className="size-9 rounded-lg text-[16px]" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[14.5px] leading-tight">
                    <PersonName name={u.name} displayName={u.display_name} />
                  </p>
                  <p className="mt-0.5 text-[12px] text-muted-foreground">{signIn(u)}</p>
                </div>
                <RoleBadge u={u} />
              </li>
            ))}
          </ul>
          <DataTable className="max-md:hidden">
            <thead>
              <tr>
                <TH>User</TH>
                <TH>Role</TH>
                <TH>Sign-in</TH>
              </tr>
            </thead>
            <tbody>
              {rows.map((u) => (
                <tr key={u.name}>
                  <TD>
                    <span className="flex items-center gap-2.5">
                      <Avatar name={u.name} /> <PersonName name={u.name} displayName={u.display_name} />
                    </span>
                  </TD>
                  <TD>
                    <RoleBadge u={u} />
                  </TD>
                  <TD className="text-[12.5px] text-muted-foreground">{signIn(u)}</TD>
                </tr>
              ))}
            </tbody>
          </DataTable>
        </>
      )}
    </PageShell>
  );
}
