import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { Navigate } from "react-router";
import { Check, ChevronsUpDown, Flame, Loader2, Plus, RotateCw, ShieldCheck, Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { DataTable, PageShell, TD, TH } from "@/components/page";
import { Avatar, EmptyState, RowSkeleton, StateDot, cookState } from "@/components/bits";
import { CopyButton, TokenDialog } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useAgents, useProjects, useV1, useV1Call, type AgentRow, type MemberRow, type TeamRow, type UserRow } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useSession } from "@/lib/session";

export function AdminOnly({ children }: { children: ReactNode }) {
  const me = useSession();
  if (!me.is_admin) return <Navigate to="/" replace />;
  return <>{children}</>;
}

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
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <form
          className="space-y-4"
          onSubmit={(e: FormEvent) => {
            e.preventDefault();
            void onSubmit();
          }}
        >
          {children}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || disabled}>
              {busy && <Loader2 className="animate-spin" />} {submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ------------------------------------------------------------------ Fires (relays)

export function FiresPage() {
  const relays = useV1<AgentRow[]>("/relays");
  const call = useV1Call();
  const now = useNow();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [token, setToken] = useState<{ name: string; token: string; title: string } | null>(null);
  const [rotating, setRotating] = useState<string | null>(null);

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
  async function rotate(n: string) {
    try {
      const r = await call<{ token: string }>("POST", `/relays/${n}/rotate`);
      setToken({ name: n, token: r.token, title: `New token for ${n}` });
    } catch (e) {
      toast.error("Could not rotate the token", { description: errorMessage(e) });
    }
  }

  return (
    <PageShell
      eyebrow="Kitchen"
      title="Fires"
      sub="A fire is a relay running on your laptop: it listens for work and wakes your local cooks. Cloud cooks are woken by a webhook instead."
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> Light a fire
        </Button>
      }
    >
      {relays.isLoading ? (
        <RowSkeleton rows={2} />
      ) : (relays.data ?? []).length === 0 ? (
        <EmptyState title="No fire is lit" action={<Button onClick={() => setCreating(true)}>Light the first fire</Button>}>
          Enroll a relay on the machine where your cooks live. Run <code className="font-mono">corntchap-cli fire enroll relay-laptop</code> there and approve it in the browser, or make one here.
        </EmptyState>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 2xl:grid-cols-3">
          {relays.data!.map((r) => {
            const st = cookState({ ...r, busy: 0 }, now);
            const lit = st === "awake";
            return (
              <article key={r.name} className="rounded-[14px] border border-border bg-card p-4" aria-label={`Fire ${r.name}`}>
                <div className="flex items-start gap-3">
                  <span className={cn("grid size-10 place-items-center rounded-lg", lit ? "bg-corn/20" : "bg-muted")}>
                    <Flame className={cn("size-5", lit ? "text-corn-text" : "text-muted-foreground")} aria-hidden="true" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <h2 className="truncate text-[18px] leading-tight">{r.name}</h2>
                    <p className="mt-0.5 flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
                      <StateDot state={lit ? "cooking" : st} className={lit ? "" : "bg-muted-foreground/50"} />
                      {lit ? "lit" : r.disabled ? "put out" : "banked"} · seen {fmtAgo(r.last_seen_at, now)}
                      {r.owner && <> · {r.owner}</>}
                    </p>
                  </div>
                </div>
                <div className="mt-3 flex items-center gap-2 rounded-lg bg-muted/50 px-3 py-2">
                  <code className="min-w-0 flex-1 truncate font-mono text-[11.5px]">corntchap-cli fire enroll {r.name}</code>
                  <CopyButton size="xs" text={`corntchap-cli fire enroll ${r.name}`} />
                </div>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button size="sm" variant="outline" onClick={() => setRotating(r.name)}>
                    <RotateCw /> Rotate token
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className={r.disabled ? "" : "text-destructive hover:text-destructive"}
                    onClick={async () => {
                      try {
                        await call("POST", `/agents/${r.name}/${r.disabled ? "enable" : "disable"}`);
                        toast.success(r.disabled ? `${r.name} lit again` : `${r.name} put out`);
                      } catch (e) {
                        toast.error("Could not change that", { description: errorMessage(e) });
                      }
                    }}
                  >
                    {r.disabled ? "Light again" : "Put out"}
                  </Button>
                </div>
              </article>
            );
          })}
        </div>
      )}

      <FormDialog open={creating} onOpenChange={setCreating} title="Light a fire" description="Creates a relay and shows its token once. Prefer the device flow on the laptop when you can." onSubmit={create} busy={busy} submitLabel="Light it" disabled={!name}>
        <div className="space-y-1.5">
          <Label htmlFor="fire-name">Name</Label>
          <Input id="fire-name" required autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="relay-laptop" pattern="[A-Za-z0-9][A-Za-z0-9._-]*" />
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
      <AlertDialog open={!!rotating} onOpenChange={(o) => !o && setRotating(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Rotate {rotating}'s token?</AlertDialogTitle>
            <AlertDialogDescription>The running relay stops being able to listen until it gets the new token.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Not now</AlertDialogCancel>
            <AlertDialogAction onClick={() => rotating && void rotate(rotating)}>Rotate it</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </PageShell>
  );
}

// ------------------------------------------------------------------ Pots (admin)

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
      ) : (projects.data ?? []).length === 0 ? (
        <EmptyState title="No pots yet" action={<Button onClick={() => setCreating(true)}>Start the first pot</Button>}>
          Make a team first if you have none, then start a pot for it.
        </EmptyState>
      ) : (
        <DataTable>
          <thead>
            <tr>
              <TH>Key</TH>
              <TH>Name</TH>
              <TH>Team</TH>
              <TH>Lease</TH>
              <TH className="text-right">Not served</TH>
            </tr>
          </thead>
          <tbody>
            {projects.data!.map((p) => (
              <tr key={p.key}>
                <TD className="font-mono text-[12.5px] font-bold">{p.key}</TD>
                <TD>
                  {p.name}
                  {p.repo_url && <span className="ml-2 font-mono text-[11px] text-muted-foreground">{p.repo_url}</span>}
                </TD>
                <TD>{p.team ?? "-"}</TD>
                <TD className="font-mono text-[12px]">{Math.round(p.lease_ttl_s / 60)} min</TD>
                <TD className="text-right font-mono text-[12.5px]">{p.open + p.claimed + p.blocked}</TD>
              </tr>
            ))}
          </tbody>
        </DataTable>
      )}
      <FormDialog open={creating} onOpenChange={setCreating} title="New pot" description="A project that tasks live in. Its key prefixes every task id." onSubmit={create} busy={busy} submitLabel="Start the pot" disabled={!key || !name || !(team || teams.data?.[0])}>
        <div className="grid grid-cols-[7rem_1fr] gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="pot-key">Key</Label>
            <Input id="pot-key" required autoFocus value={key} onChange={(e) => setKey(e.target.value.toUpperCase())} placeholder="PAY" className="font-mono" pattern="[A-Za-z][A-Za-z0-9]{0,9}" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="pot-name">Name</Label>
            <Input id="pot-name" required value={name} onChange={(e) => setName(e.target.value)} placeholder="Payments" />
          </div>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="pot-team">Team</Label>
          <Select value={team || teams.data?.[0]?.name || ""} onValueChange={setTeam}>
            <SelectTrigger id="pot-team" className="w-full">
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
        <div className="grid grid-cols-[1fr_8rem] gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="pot-repo">Repository (optional)</Label>
            <Input id="pot-repo" type="url" value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="https://github.com/acme/payments" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="pot-ttl">Lease (s)</Label>
            <Input id="pot-ttl" type="number" min={30} max={86400} value={ttl} onChange={(e) => setTtl(e.target.value)} />
          </div>
        </div>
      </FormDialog>
    </PageShell>
  );
}

// ------------------------------------------------------------------ Teams (admin)

export function TeamsAdminPage() {
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
      eyebrow="Admin"
      title="Teams"
      sub="Teams decide who sees which pots. Members can be people or cooks."
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> New team
        </Button>
      }
    >
      {teams.isLoading ? (
        <RowSkeleton rows={3} />
      ) : (teams.data ?? []).length === 0 ? (
        <EmptyState title="No teams yet" action={<Button onClick={() => setCreating(true)}>Make the first team</Button>}>
          A team owns pots and has members. Start with the people who cook together.
        </EmptyState>
      ) : (
        <div className="space-y-4">
          {teams.data!.map((t) => (
            <TeamCard key={t.name} team={t} onAdd={() => setAdding(t.name)} />
          ))}
        </div>
      )}
      <FormDialog open={creating} onOpenChange={setCreating} title="New team" description="You become its owner." onSubmit={create} busy={busy} submitLabel="Create team" disabled={!name}>
        <div className="space-y-1.5">
          <Label htmlFor="team-name">Name</Label>
          <Input id="team-name" required autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="acme" />
        </div>
      </FormDialog>
      <AddMember team={adding} onClose={() => setAdding(null)} />
    </PageShell>
  );
}

function TeamCard({ team, onAdd }: { team: TeamRow; onAdd: () => void }) {
  const members = useV1<MemberRow[]>(`/teams/${team.name}/members`);
  const call = useV1Call();
  return (
    <section className="rounded-[14px] border border-border bg-card" aria-label={`Team ${team.name}`}>
      <div className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3">
        <h2 className="text-[18px]">{team.name}</h2>
        <span className="font-mono text-[11.5px] text-muted-foreground">
          {team.members} members · pots {team.projects.length ? team.projects.join(", ") : "none"}
        </span>
        <Button size="sm" variant="outline" className="ml-auto" onClick={onAdd}>
          <UserPlus /> Add member
        </Button>
      </div>
      <ul className="divide-y divide-border">
        {members.isLoading && <li className="px-4 py-3 text-muted-foreground">Loading…</li>}
        {(members.data ?? []).map((m) => (
          <li key={m.name} className="flex items-center gap-3 px-4 py-2.5">
            <Avatar name={m.name} className="size-7 text-[13px]" />
            <span className="font-medium">{m.name}</span>
            <span className="rounded-full bg-muted px-2 py-px text-[11px] text-muted-foreground">{m.kind}</span>
            <span className="font-mono text-[11px] text-muted-foreground">{m.role}</span>
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
          </li>
        ))}
      </ul>
    </section>
  );
}

interface Candidate {
  /** What the API takes as `principal`: a user's login or a cook's name. */
  principal: string;
  label: string;
  kind: "user" | "cook";
}

function AddMember({ team, onClose }: { team: string | null; onClose: () => void }) {
  const call = useV1Call();
  const open = !!team;
  const users = useV1<UserRow[]>("/users", { enabled: open });
  const cooks = useAgents();
  const members = useV1<MemberRow[]>(`/teams/${team}/members`, { enabled: open });
  const [who, setWho] = useState<Candidate | null>(null);
  const [pick, setPick] = useState(false);
  const [role, setRole] = useState("member");
  const [busy, setBusy] = useState(false);

  const { people, cookList } = useMemo(() => {
    const taken = new Set((members.data ?? []).map((m) => m.name));
    const people: Candidate[] = (users.data ?? [])
      .filter((u) => !u.disabled && !taken.has(u.name) && !(u.login && taken.has(u.login)))
      .map((u) => ({ principal: u.login ?? u.name, label: u.login ?? u.name, kind: "user" as const }));
    const cookList: Candidate[] = (cooks.data ?? [])
      .filter((c) => !c.disabled && !taken.has(c.name))
      .map((c) => ({ principal: c.name, label: c.name, kind: "cook" as const }));
    return { people, cookList };
  }, [users.data, cooks.data, members.data]);
  const loading = users.isLoading || cooks.isLoading || members.isLoading;

  useEffect(() => {
    if (!open) {
      setWho(null);
      setRole("member");
      setPick(false);
    }
  }, [open]);

  return (
    <FormDialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={`Add to ${team}`}
      description="Pick a person or a cook that is not on this team yet."
      busy={busy}
      submitLabel="Add"
      disabled={!who}
      onSubmit={async () => {
        if (!who) return;
        setBusy(true);
        try {
          await call("POST", `/teams/${team}/members`, { principal: who.principal, role });
          toast.success(`${who.label} joined ${team}`);
          onClose();
        } catch (e) {
          toast.error("Could not add them", { description: errorMessage(e) });
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="space-y-1.5">
        <Label htmlFor="mem-who">Member</Label>
        <Popover open={pick} onOpenChange={setPick}>
          <PopoverTrigger asChild>
            <Button
              id="mem-who"
              type="button"
              variant="outline"
              role="combobox"
              aria-expanded={pick}
              className="w-full justify-between px-3 font-normal"
            >
              {who ? (
                <span className="flex min-w-0 items-center gap-2">
                  <Avatar name={who.label} className="size-5 text-[10px]" />
                  <span className="truncate">{who.label}</span>
                  <span className="rounded-full bg-muted px-2 py-px text-[11px] text-muted-foreground">{who.kind === "cook" ? "cook" : "person"}</span>
                </span>
              ) : (
                <span className="text-muted-foreground">{loading ? "Loading…" : "Search people and cooks"}</span>
              )}
              <ChevronsUpDown className="size-4 shrink-0 opacity-50" aria-hidden="true" />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-(--radix-popover-trigger-width) p-0">
            <Command>
              <CommandInput placeholder="Search by login or name" />
              <CommandList>
                <CommandEmpty>{loading ? "Loading…" : "Nobody left to add."}</CommandEmpty>
                {people.length > 0 && (
                  <CommandGroup heading="People">
                    {people.map((c) => (
                      <CommandItem
                        key={`u:${c.principal}`}
                        value={`person ${c.label}`}
                        onSelect={() => {
                          setWho(c);
                          setPick(false);
                        }}
                      >
                        <Avatar name={c.label} className="size-5 text-[10px]" />
                        <span className="min-w-0 flex-1 truncate">{c.label}</span>
                        <span className="text-[11px] text-muted-foreground">person</span>
                        {who?.kind === c.kind && who.principal === c.principal && <Check className="size-4" aria-hidden="true" />}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                )}
                {cookList.length > 0 && (
                  <CommandGroup heading="Cooks">
                    {cookList.map((c) => (
                      <CommandItem
                        key={`c:${c.principal}`}
                        value={`cook ${c.label}`}
                        onSelect={() => {
                          setWho(c);
                          setPick(false);
                        }}
                      >
                        <Avatar name={c.label} className="size-5 text-[10px]" />
                        <span className="min-w-0 flex-1 truncate">{c.label}</span>
                        <span className="text-[11px] text-muted-foreground">cook</span>
                        {who?.kind === c.kind && who.principal === c.principal && <Check className="size-4" aria-hidden="true" />}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                )}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="mem-role">Role</Label>
        <Select value={role} onValueChange={setRole}>
          <SelectTrigger id="mem-role" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="member">Member</SelectItem>
            <SelectItem value="owner">Owner</SelectItem>
          </SelectContent>
        </Select>
      </div>
    </FormDialog>
  );
}

// ------------------------------------------------------------------ Users (admin)

export function UsersAdminPage() {
  const users = useV1<UserRow[]>("/users");
  const call = useV1Call();
  const [creating, setCreating] = useState(false);
  const [login, setLogin] = useState("");
  const [admin, setAdmin] = useState(false);
  const [busy, setBusy] = useState(false);

  async function create() {
    setBusy(true);
    try {
      await call("POST", "/users", { login, is_admin: admin });
      toast.success(`${login} can sign in`);
      setCreating(false);
      setLogin("");
      setAdmin(false);
    } catch (e) {
      toast.error("Could not add that user", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageShell
      eyebrow="Admin"
      title="Users"
      sub="People who can sign in through your identity provider. Users are also created the first time they sign in."
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> Add user
        </Button>
      }
    >
      {users.isLoading ? (
        <RowSkeleton rows={3} />
      ) : (
        <DataTable>
          <thead>
            <tr>
              <TH>User</TH>
              <TH>Login</TH>
              <TH>Role</TH>
              <TH>Sign-in</TH>
            </tr>
          </thead>
          <tbody>
            {(users.data ?? []).map((u) => (
              <tr key={u.name}>
                <TD>
                  <span className="flex items-center gap-2.5">
                    <Avatar name={u.name} /> <span className="font-medium">{u.name}</span>
                  </span>
                </TD>
                <TD className="font-mono text-[12px] text-muted-foreground">{u.login ?? "-"}</TD>
                <TD>
                  {u.is_admin ? (
                    <span className="inline-flex items-center gap-1 rounded-full bg-primary/15 px-2 py-0.5 text-[11.5px] font-medium text-primary">
                      <ShieldCheck className="size-3" aria-hidden="true" /> admin
                    </span>
                  ) : (
                    <span className="text-muted-foreground">member</span>
                  )}
                </TD>
                <TD className="text-[12.5px] text-muted-foreground">
                  {u.disabled ? "disabled" : u.linked ? "linked" : "waiting for first sign-in"}
                </TD>
              </tr>
            ))}
          </tbody>
        </DataTable>
      )}
      <FormDialog open={creating} onOpenChange={setCreating} title="Add a user" description="Pre-provision a login so you can add it to a team before it ever signs in." onSubmit={create} busy={busy} submitLabel="Add user" disabled={!login}>
        <div className="space-y-1.5">
          <Label htmlFor="usr-login">Login</Label>
          <Input id="usr-login" required autoFocus value={login} onChange={(e) => setLogin(e.target.value)} placeholder="dev" />
        </div>
        <div className="flex items-center justify-between rounded-lg border border-border bg-muted/40 px-3 py-2.5">
          <Label htmlFor="usr-admin">Admin</Label>
          <Switch id="usr-admin" checked={admin} onCheckedChange={setAdmin} />
        </div>
      </FormDialog>
    </PageShell>
  );
}
