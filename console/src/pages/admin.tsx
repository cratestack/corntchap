import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { Link, Navigate, useNavigate, useSearchParams } from "react-router";
import { ChevronRight, Flame, Loader2, Pencil, Plus, Power, RotateCw, Search, ShieldCheck, Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import type { PrincipalView, ProjectSummary, TeamView, UserView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { Avatar, FormSheetBody, FormSheetFoot, FormSheetHead, Group, GroupRow, PersonName, formSheetClass } from "@/components/bits";
import { CopyLine, TokenDialog } from "@/components/secret-dialog";
import { ConfirmDialog, DeleteDialog, RenameDialog } from "@/components/rename-delete";
import { PotDeleteDialog, PotRenameDialog } from "@/components/pot-options";
import { FilterBar, type FacetOption } from "@/components/filter-bar";
import { ListPage } from "@/components/list-page";
import { PageTools } from "@/components/page-tools";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { VirtualListbox } from "@/components/ui/virtual-listbox";
import { type VirtualTableColumn } from "@/components/ui/virtual-table";
import { useProcCall, useV1Call } from "@/lib/data";
import { useCooksSource, useFiresSource, usePotsSource, useTeamMembersSource, useTeamsSource, useUsersSource } from "@/lib/sources";
import { DISABLED_OPTIONS, disabledArg, principalState, staticFacet, useFilteredSource, useMappedSource, useTeamFacet } from "@/lib/people";
import { useUrlFilters, type UrlFilterSchema } from "@/lib/url-filters";
import { useStoreValue } from "@/lib/use-store-value";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { canManageTeam, canOrchestrate, isSuperAdmin, roleLabel, useSession, type Role } from "@/lib/session";

/** The pages only a super-admin sees (people, forges). */
export function AdminOnly({ children }: { children: ReactNode }) {
  const me = useSession();
  if (!isSuperAdmin(me)) return <Navigate to="/" replace />;
  return <>{children}</>;
}

/** The pages an orchestrator (or a super-admin) sees: pots of their own teams. */
export function OrchestratorOnly({ children }: { children: ReactNode }) {
  const me = useSession();
  if (!canOrchestrate(me)) return <Navigate to="/" replace />;
  return <>{children}</>;
}

/** A form in a dialog: a bottom sheet on phones (the body scrolls, the button stays put under the keyboard). */
export function FormDialog({
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
export function fireState(r: Pick<PrincipalView, "disabled" | "last_seen_at">, now: number) {
  const lit = principalState({ ...r, busy: 0 }, now) === "awake";
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
export function FireManage({ fire }: { fire: Pick<PrincipalView, "name" | "disabled"> }) {
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

const NO_FILTERS: UrlFilterSchema = {};
const FIRE_FILTERS = { lit: { type: "string" } } as const satisfies UrlFilterSchema;
/** Lit: seen in the last five minutes and not put out; Out: the others. */
const LIT_OPTIONS = [
  { value: "true", label: "Lit" },
  { value: "false", label: "Out" },
] as const;
const firePk = (r: PrincipalView) => r.name;

export function FiresPage() {
  const f = useUrlFilters(FIRE_FILTERS);
  const fires = useFiresSource(f.q, f.values.lit ? { lit: f.values.lit === "true" } : {});
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

  return (
    <>
      <ListPage<PrincipalView>
        title="Fires"
        actions={
          <>
            <PageTools />
            <Button onClick={() => setCreating(true)}>
              <Plus /> Light a fire
            </Button>
          </>
        }
        filters={
          <div className="space-y-3">
            <p className="max-w-2xl text-[13.5px] text-muted-foreground">
              A fire is a relay running on your laptop: it listens for work and wakes your local cooks. Cloud cooks are woken by a routine, a GitHub comment or a webhook instead.
            </p>
            <FilterBar
              q={f.q}
              onQ={f.setQ}
              facets={[staticFacet("lit", "Fire", LIT_OPTIONS)]}
              values={{ lit: f.values.lit }}
              onChange={(p) => f.set(p as never)}
              onClear={f.clear}
              searchPlaceholder="Find a fire"
            />
          </div>
        }
        source={fires}
        itemKey={firePk}
        renderItem={(r) => {
          const { lit, label } = fireState(r, now);
          return (
            <div className={cn(lit && "bg-corn/5")}>
              <Link
                to={`/fires/${r.name}`}
                aria-label={`Fire ${r.name}, ${label}`}
                className="flex min-h-[4.5rem] items-center gap-3 p-4 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted"
              >
                <FireTile lit={lit} />
                <div className="min-w-0 flex-1">
                  <h2 className="truncate font-display text-[18px] leading-tight">{r.name}</h2>
                  <p className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
                    seen {fmtAgo(r.last_seen_at ?? null, now)}
                    {r.owner && <> · {r.owner}</>}
                  </p>
                </div>
                <FireBadge lit={lit} label={label} />
                <ChevronRight className="size-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
              </Link>
              <div className="border-t border-border bg-muted/30 p-2.5">
                <CopyLine text={`corntchap-cli fire enroll ${r.name}`} />
              </div>
            </div>
          );
        }}
        empty={
          f.q || f.active > 0 ? (
            <>No fire matches these filters.</>
          ) : (
            <>
              No fire is lit. Enroll a relay on the machine where your cooks live: run <code className="font-mono">corntchap-cli fire enroll relay-laptop</code> there and approve it in the browser, or{" "}
              <Button variant="link" className="h-auto p-0" onClick={() => setCreating(true)}>
                make one here
              </Button>
              .
            </>
          )
        }
      />

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
    </>
  );
}

// ------------------------------------------------------------------ Pots (admin)

const leaseText = (s: number) => (s >= 120 ? `${Math.round(s / 60)} min` : `${s} s`);
const POT_FILTERS = { team: { type: "string" } } as const satisfies UrlFilterSchema;
const potPk = (p: ProjectSummary) => p.key;
const teamPk = (t: TeamView) => t.name;
const teamText = (t: TeamView) => t.name;

export function PotsAdminPage() {
  const me = useSession();
  const f = useUrlFilters(POT_FILTERS);
  const teamFacet = useTeamFacet();
  const pots = usePotsSource({ q: f.q, ...(f.values.team ? { team: f.values.team } : {}) });
  const call = useV1Call();
  const [params, setParams] = useSearchParams();
  const [creating, setCreating] = useState(params.get("new") === "1");
  useEffect(() => {
    if (params.get("new") === "1") {
      const n = new URLSearchParams(params);
      n.delete("new");
      setParams(n, { replace: true });
    }
  }, [params, setParams]);
  const [team, setTeam] = useState("");
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [repo, setRepo] = useState("");
  const [ttl, setTtl] = useState("900");
  const [busy, setBusy] = useState(false);
  const [potDialog, setPotDialog] = useState<{ kind: "rename" | "delete"; key: string } | null>(null);
  const dialogPot = pots.items.find((p) => p.key === potDialog?.key);

  // pots go into teams you own (a super-admin: any team)
  const newTeamQ = useStoreValue("");
  const allTeams = useTeamsSource(newTeamQ.value, { enabled: creating });
  const manageable = useCallback((t: TeamView) => canManageTeam(me, t.role), [me]);
  const teams = useFilteredSource(allTeams, manageable);
  const chosenTeam = team || teams.items[0]?.name || "";

  async function create() {
    setBusy(true);
    try {
      await call("POST", "/projects", {
        team: chosenTeam,
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

  const columns: VirtualTableColumn<ProjectSummary>[] = [
    {
      id: "key",
      header: "Key",
      className: "font-mono text-[12.5px] font-bold",
      cell: (p) => (
        <Link to={`/pots/${p.key}/about`} className="underline-offset-2 hover:underline focus-visible:underline">
          {p.key}
        </Link>
      ),
    },
    {
      id: "name",
      header: "Name",
      cell: (p) => (
        <>
          {p.name}
          {p.repo_url && <span className="ml-2 font-mono text-[11px] break-all text-muted-foreground">{p.repo_url}</span>}
        </>
      ),
    },
    { id: "team", header: "Team", cell: (p) => p.team ?? "-" },
    { id: "lease", header: "Lease", className: "font-mono text-[12px]", cell: (p) => `${Math.round(p.lease_ttl_s / 60)} min` },
    { id: "unserved", header: "Not served", className: "text-right font-mono text-[12.5px]", cell: (p) => p.open + p.claimed + p.blocked },
    {
      id: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "whitespace-nowrap text-right",
      cell: (p) => (
        <>
          <Button size="icon-sm" variant="ghost" aria-label={`Rename ${p.key}`} onClick={() => setPotDialog({ kind: "rename", key: p.key })}>
            <Pencil />
          </Button>
          <Button size="icon-sm" variant="ghost" className="text-destructive hover:text-destructive" aria-label={`Delete ${p.key}`} onClick={() => setPotDialog({ kind: "delete", key: p.key })}>
            <Trash2 />
          </Button>
        </>
      ),
    },
  ];

  return (
    <>
      <ListPage<ProjectSummary>
        title="Pots"
        actions={
          <>
            <PageTools />
            <Button onClick={() => setCreating(true)}>
              <Plus /> New pot
            </Button>
          </>
        }
        filters={
          <div className="space-y-3">
            <p className="max-w-2xl text-[13.5px] text-muted-foreground">Pots are projects: a key like PAY, a team that may see them and a default lease length for scooped tasks.</p>
            <FilterBar q={f.q} onQ={f.setQ} facets={[teamFacet]} values={{ team: f.values.team }} onChange={(p) => f.set(p as never)} onClear={f.clear} searchPlaceholder="Find a pot" />
          </div>
        }
        source={pots}
        itemKey={potPk}
        columns={columns}
        // phones: two-line rows, a tap opens the pot (rename and delete live there)
        renderItem={(p) => (
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
        )}
        empty={
          f.q || f.values.team ? (
            <>No pot matches these filters.</>
          ) : (
            <>
              No pots yet. Make a team first if you have none, then{" "}
              <Button variant="link" className="h-auto p-0" onClick={() => setCreating(true)}>
                start the first pot
              </Button>
              .
            </>
          )
        }
      />
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
        disabled={!key || !name || !chosenTeam}
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
          <VirtualCombobox<TeamView>
            id="pot-team"
            label="Team"
            placeholder="Make a team first"
            source={teams}
            onQuery={newTeamQ.set}
            itemKey={teamPk}
            itemText={teamText}
            value={chosenTeam || null}
            onChange={(v) => setTeam(v ?? "")}
            className="h-11"
          />
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
    </>
  );
}

// ------------------------------------------------------------------ Teams (admin)

export function TeamsAdminPage() {
  const me = useSession();
  const f = useUrlFilters(NO_FILTERS);
  const teams = useTeamsSource(f.q);
  const call = useV1Call();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const addingRole = teams.items.find((t) => t.name === adding)?.role;

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
    <>
      <ListPage<TeamView>
        title="Teams"
        actions={
          <>
            <PageTools />
            {canOrchestrate(me) && (
              <Button onClick={() => setCreating(true)}>
                <Plus /> New team
              </Button>
            )}
          </>
        }
        filters={
          <div className="space-y-3">
            <p className="max-w-2xl text-[13.5px] text-muted-foreground">Teams decide who sees which pots. Their chefs can be people or cooks.</p>
            <FilterBar q={f.q} onQ={f.setQ} facets={[]} values={{}} onChange={() => {}} onClear={f.clear} searchPlaceholder="Find a team" />
          </div>
        }
        source={teams}
        itemKey={teamPk}
        renderItem={(t) => <TeamCard team={t} canManage={canManageTeam(me, t.role)} onAdd={() => setAdding(t.name)} />}
        empty={
          f.q ? (
            <>No team is named like “{f.q}”.</>
          ) : canOrchestrate(me) ? (
            <>
              No teams yet. A team owns pots and has chefs: start with the people who cook together.{" "}
              <Button variant="link" className="h-auto p-0" onClick={() => setCreating(true)}>
                Make the first team
              </Button>
              .
            </>
          ) : (
            <>You are in no team yet. An orchestrator creates teams and adds you.</>
          )
        }
      />
      <FormDialog open={creating} onOpenChange={setCreating} title="New team" description="You become its owner." onSubmit={create} busy={busy} submitLabel="Create team" disabled={!name}>
        <div className="space-y-1.5">
          <Label htmlFor="team-name">Name</Label>
          <Input id="team-name" required autoFocus autoCapitalize="none" value={name} onChange={(e) => setName(e.target.value)} placeholder="acme" className="h-11" />
        </div>
      </FormDialog>
      <AddMember team={adding} onClose={() => setAdding(null)} ownCooksOnly={!!adding && !canManageTeam(me, addingRole)} />
    </>
  );
}

const MEMBER_AVATARS = 6;

function TeamCard({ team, canManage, onAdd }: { team: TeamView; canManage: boolean; onAdd: () => void }) {
  const members = useTeamMembersSource(team.name);
  const list = members.items;
  return (
    <div>
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
          {list.slice(0, MEMBER_AVATARS).map((m) => (
            <Avatar key={m.name} name={m.name} className="-ml-1.5 size-7 rounded-full text-[12px] ring-2 ring-card first:ml-0" />
          ))}
          {team.members > MEMBER_AVATARS && <span className="ml-1.5 font-mono text-[11px] text-muted-foreground">+{team.members - MEMBER_AVATARS}</span>}
          {members.isLoading && <span className="text-[12px] text-muted-foreground">Loading…</span>}
        </div>
        <Button variant="outline" size="sm" onClick={onAdd}>
          <UserPlus /> {canManage ? "Add chef" : "Add my cook"}
        </Button>
      </div>
    </div>
  );
}

interface Candidate {
  /** What the API takes as `principal`: a person's handle or a cook's name. */
  principal: string;
  label: string;
  display?: string | null | undefined;
  kind: "user" | "cook";
  /** People: their role (an owner of a team must be an orchestrator or a super-admin). */
  role?: Role;
}

const personCandidate = (u: UserView): Candidate => ({ principal: u.name, label: u.name, display: u.display_name, kind: "user", role: u.role as Role });
const cookCandidate = (c: PrincipalView): Candidate => ({ principal: c.name, label: c.name, kind: "cook" });
const candidateKey = (c: Candidate) => `${c.kind}:${c.principal}`;

/** `ownCooksOnly`: a plain chef of the team, who may add only their own cooks (as chefs). */
export function AddMember({ team, onClose, ownCooksOnly = false }: { team: string | null; onClose: () => void; ownCooksOnly?: boolean }) {
  const call = useV1Call();
  const me = useSession();
  const open = !!team;
  // people: everything for a super-admin, the directory (handle, name, role) for an orchestrator; a plain chef adds only their own cooks
  const withPeople = canOrchestrate(me) && !ownCooksOnly;
  const [who, setWho] = useState<Candidate | null>(null);
  const [kind, setKind] = useState<"user" | "cook">("user");
  const [query, setQuery] = useState("");
  const [role, setRole] = useState("member");
  const [busy, setBusy] = useState(false);
  const showing = withPeople ? kind : "cook";

  // each list is paged and searched by the server, and holds only those who are not in the team yet
  const notInTeam = team ?? "";
  const people = useMappedSource(useUsersSource({ q: query, disabled: false, notInTeam, enabled: open && showing === "user" }), personCandidate);
  const cooks = useMappedSource(useCooksSource({ q: query, disabled: false, notInTeam, ...(ownCooksOnly ? { owner: me.name } : {}), enabled: open && showing === "cook" }), cookCandidate);
  const source = showing === "user" ? people : cooks;

  useEffect(() => {
    if (!open) {
      setWho(null);
      setKind("user");
      setRole("member");
      setQuery("");
    }
  }, [open]);
  const principal = who?.principal ?? "";
  const ownerOk = who?.kind === "user" && who.role !== "member";

  return (
    <FormDialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={ownCooksOnly ? `Add my cook to ${team}` : `Add to ${team}`}
      description={ownCooksOnly ? "Pick one of your cooks that is not on this team yet. It joins as a chef." : "Pick a person or a cook that is not on this team yet. Only orchestrators and super-admins can be owners."}
      busy={busy}
      submitLabel="Add"
      disabled={!principal || (role === "owner" && !!who && !ownerOk)}
      onSubmit={async () => {
        if (!principal) return;
        setBusy(true);
        try {
          await call("POST", `/teams/${team}/members`, { principal, role: ownCooksOnly ? "member" : role });
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
        {withPeople && (
          <div role="radiogroup" aria-label="Kind of chef" className="grid grid-cols-2 gap-2">
            {(
              [
                { id: "user", label: "People" },
                { id: "cook", label: "Cooks" },
              ] as const
            ).map((k) => (
              <button
                key={k.id}
                type="button"
                role="radio"
                aria-checked={kind === k.id}
                onClick={() => {
                  setKind(k.id);
                  setWho(null);
                }}
                className={cn("h-10 rounded-xl border text-[13.5px] font-medium transition-colors", kind === k.id ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50")}
              >
                {k.label}
              </button>
            ))}
          </div>
        )}
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            id="mem-search"
            type="search"
            autoComplete="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={showing === "user" ? "Search people" : ownCooksOnly ? "Search your cooks" : "Search cooks"}
            className="h-11 pl-9"
          />
        </div>
        <VirtualListbox<Candidate>
          source={source}
          label={showing === "user" ? "People" : "Cooks"}
          itemKey={candidateKey}
          selected={(c) => who?.kind === c.kind && who.principal === c.principal}
          onPick={(c) => setWho(who?.kind === c.kind && who.principal === c.principal ? null : c)}
          empty={<p className="px-3.5 py-3 text-[13px] text-muted-foreground">Nobody left to add.</p>}
          className="rounded-xl border border-border"
          rows={5}
          renderItem={(c) => (
            <span className="flex min-h-10 w-full items-center gap-2.5 px-2">
              <Avatar name={c.label} className="size-7 text-[12px]" />
              {c.kind === "user" ? <PersonName name={c.label} displayName={c.display} className="min-w-0 flex-1 truncate" /> : <span className="min-w-0 flex-1 truncate font-medium">{c.label}</span>}
              <span className="rounded-full bg-muted px-2 py-px text-[11px] text-muted-foreground">{c.kind === "cook" ? "cook" : "person"}</span>
            </span>
          )}
        />
      </div>
      {!ownCooksOnly && (
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
                disabled={r.id === "owner" && !!who && !ownerOk}
                onClick={() => setRole(r.id)}
                className={cn("h-11 rounded-xl border text-[13.5px] font-medium transition-colors disabled:opacity-50", role === r.id ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50")}
              >
                {r.label}
              </button>
            ))}
          </div>
        </div>
      )}
    </FormDialog>
  );
}

// ------------------------------------------------------------------ Users (admin)

const ROLES: { id: Role; label: string }[] = [
  { id: "super_admin", label: "Super-admin" },
  { id: "orchestrator", label: "Orchestrator" },
  { id: "member", label: "Member" },
];
const ROLE_OPTIONS: readonly FacetOption[] = ROLES.map((r) => ({ value: r.id, label: r.label }));

/** A person's role as a select: changing it is a manual setting the identity provider no longer overrides. */
function RoleSelect({ u }: { u: UserView }) {
  const call = useV1Call();
  const [busy, setBusy] = useState(false);
  async function change(role: string) {
    setBusy(true);
    try {
      await call("PATCH", `/users/${u.name}`, { role });
      toast.success(`${u.name} is now ${roleLabel(role as Role)}`);
    } catch (e) {
      toast.error("Could not change the role", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Select value={u.role} onValueChange={(v) => void change(v)} disabled={busy}>
      <SelectTrigger aria-label={`Role of ${u.name}`} className="h-9 w-40">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {ROLES.map((r) => (
          <SelectItem key={r.id} value={r.id}>
            {r.id === "super_admin" && <ShieldCheck className="size-3.5" aria-hidden="true" />} {r.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

const signIn = (u: UserView) => (u.disabled ? "disabled" : u.linked ? "signed in" : "local (no sign-in)");
const USER_FILTERS = { role: { type: "string" }, disabled: { type: "string" } } as const satisfies UrlFilterSchema;
const userPk = (u: UserView) => u.name;

const USER_COLUMNS: VirtualTableColumn<UserView>[] = [
  {
    id: "user",
    header: "User",
    cell: (u) => (
      <span className="flex items-center gap-2.5">
        <Avatar name={u.name} /> <PersonName name={u.name} displayName={u.display_name} />
      </span>
    ),
  },
  { id: "role", header: "Role", cell: (u) => <RoleSelect u={u} /> },
  { id: "signin", header: "Sign-in", cell: (u) => signIn(u), className: "text-[12.5px] text-muted-foreground" },
];

export function UsersAdminPage() {
  const f = useUrlFilters(USER_FILTERS);
  const facets = useMemo(() => [staticFacet("role", "Role", ROLE_OPTIONS), staticFacet("disabled", "Sign-in", DISABLED_OPTIONS)], []);
  const users = useUsersSource({ q: f.q, ...(f.values.role ? { role: f.values.role } : {}), ...disabledArg(f.values.disabled) });
  const filtered = !!f.q || f.active > 0;

  return (
    <ListPage<UserView>
      title="Users"
      actions={<PageTools />}
      filters={
        <div className="space-y-3">
          <p className="max-w-2xl text-[13.5px] text-muted-foreground">
            People who have signed in through your identity provider. Each gets a kitchen handle at their first sign-in and can change it from the account menu. A role set here sticks: the
            identity provider no longer changes it.
          </p>
          <FilterBar
            q={f.q}
            onQ={f.setQ}
            facets={facets}
            values={{ role: f.values.role, disabled: f.values.disabled }}
            onChange={(p) => f.set(p as never)}
            onClear={f.clear}
            searchPlaceholder="Find a person by handle or name"
          />
        </div>
      }
      source={users}
      itemKey={userPk}
      columns={USER_COLUMNS}
      renderItem={(u) => (
        // on the narrowest phones the role picker drops under the name rather than squeezing it to a few letters
        <div className="flex min-h-16 flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3">
          <Avatar name={u.name} className="size-9 rounded-lg text-[16px]" />
          <div className="min-w-32 flex-1 basis-32">
            <p className="truncate text-[14.5px] leading-tight">
              <PersonName name={u.name} displayName={u.display_name} />
            </p>
            <p className="mt-0.5 text-[12px] text-muted-foreground">{signIn(u)}</p>
          </div>
          <div className="ml-auto">
            <RoleSelect u={u} />
          </div>
        </div>
      )}
      empty={filtered ? <>No one matches these filters.</> : <>No users yet. People show up here after their first sign-in.</>}
    />
  );
}
