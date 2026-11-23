import { useEffect, useMemo, useState, type FormEvent } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { ChefHat, ChevronDown, Loader2, Minus, Plug, Plus, Power, RotateCw, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { PageShell } from "@/components/page";
import { Avatar, EmptyState, LabelChip, RowSkeleton, StateDot, STATE_LABEL, cookState } from "@/components/bits";
import { StationList } from "@/components/stations";
import { CookUsage, fmtTokens } from "@/components/usage";
import { McpDialog, TokenDialog } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
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
import {
  useAgents,
  useClaimedTasks,
  useProc,
  useProcCall,
  useProjects,
  useV1,
  useInvalidateAll,
  useV1Call,
  type AgentRow,
  type TeamRow,
  type WakeTargetRow,
} from "@/lib/data";
import type { StationView } from "@corntchap/api";
import { client, errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { KIND_LABEL } from "@/lib/snippets";
import { useSession } from "@/lib/session";
import { useOpenTask } from "@/lib/nav";

export function CooksPage() {
  const agents = useAgents();
  const projects = useProjects();
  const claimed = useClaimedTasks(projects.data);
  const stations = useProc("stationList", {});
  const usage = useProc("usageReport", { days: 7 });
  const [creating, setCreating] = useState(false);
  const [token, setToken] = useState<{ name: string; kind: string | null; token: string; title: string; hint?: string } | null>(null);
  const loc = useLocation();

  useEffect(() => {
    if (loc.hash && agents.data) {
      document.getElementById(`cook-${loc.hash.slice(1)}`)?.scrollIntoView({ block: "center" });
    }
  }, [loc.hash, agents.data]);

  const claimedTasks = claimed.tasks;
  const current = useMemo(() => {
    const m = new Map<string, (typeof claimedTasks)[number]>();
    for (const t of claimedTasks) if (t.claimed_by && t.lease_live) m.set(t.claimed_by, t);
    return m;
  }, [claimedTasks]);

  return (
    <PageShell
      eyebrow="Kitchen"
      title="Cooks"
      sub={
        <>
          The agents that scoop and serve your tasks. Each one has its own token, a lease limit and, if it sleeps, a wake target.
          {usage.data && (
            <span className="mt-1 block" title={usage.data.note}>
              Tokens sent (7 days): <span className="font-mono font-bold">~{fmtTokens(usage.data.totals.tokens)}</span>, an estimate of
              what corntchap put in your cooks' context.
            </span>
          )}
        </>
      }
      actions={
        <Button onClick={() => setCreating(true)}>
          <Plus /> New cook
        </Button>
      }
    >
      {agents.isLoading ? (
        <RowSkeleton rows={3} />
      ) : agents.error ? (
        <EmptyState band={false} title="Could not reach the cooks">{errorMessage(agents.error)}</EmptyState>
      ) : agents.data?.length === 0 ? (
        <EmptyState title="No cooks in the kitchen" action={<Button onClick={() => setCreating(true)}>Hire the first cook</Button>}>
          A cook is an agent (Claude Code, Codex, OpenCode or any MCP client). Make one, copy its config, and it can start scooping.
        </EmptyState>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 2xl:grid-cols-3">
          {agents.data!.map((a) => (
            <CookCard
              key={a.name}
              a={a}
              current={current.get(a.name)?.id ?? null}
              stations={(stations.data ?? []).filter((s) => s.cook === a.name)}
              onToken={(t) => setToken({ name: a.name, kind: a.agent_kind, ...t })}
            />
          ))}
        </div>
      )}
      <CreateCook
        open={creating}
        onOpenChange={setCreating}
        onCreated={(c) =>
          setToken({
            name: c.name,
            kind: c.kind,
            token: c.token,
            title: `${c.name} is in the kitchen`,
            hint: "Put the token in the MCP config below, or keep it in an env var.",
          })
        }
      />
      <TokenDialog
        open={!!token}
        onOpenChange={(o) => !o && setToken(null)}
        title={token?.title ?? ""}
        name={token?.name ?? ""}
        token={token?.token ?? null}
        kind={token?.kind ?? null}
        {...(token?.hint ? { hint: token.hint } : {})}
      />
    </PageShell>
  );
}

const GRANTS: { id: string; label: string; hint: string }[] = [
  { id: "plan", label: "Plan", hint: "Edit, toss, restore and unblock any task in its teams; tick items without a lease" },
  { id: "manage", label: "Manage", hint: "Act as its owner on teams, pots, cooks, fires and wake targets" },
  { id: "admin", label: "Admin", hint: "Its owner's admin powers (only if the owner is an admin)" },
];

/** What a cook may do beyond working its tasks. A cook never exceeds its owner and cannot change this itself. */
function Grants({ a }: { a: AgentRow }) {
  const call = useV1Call();
  const [busy, setBusy] = useState<string | null>(null);
  const have = new Set(a.grants ?? []);
  async function toggle(id: string) {
    const next = have.has(id) ? [...have].filter((g) => g !== id) : [...have, id];
    setBusy(id);
    try {
      await call("POST", `/agents/${a.name}/grants`, { grants: next });
      toast.success(have.has(id) ? `${a.name} may no longer ${id}` : `${a.name} may now ${id}`);
    } catch (e) {
      toast.error("Could not change what this cook may do", { description: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  }
  return (
    <div className="flex flex-wrap gap-1.5" role="group" aria-label={`What ${a.name} may also do`}>
      {GRANTS.map((g) => {
        const on = have.has(g.id);
        return (
          <button
            key={g.id}
            type="button"
            title={g.hint}
            aria-pressed={on}
            disabled={!!busy}
            onClick={() => void toggle(g.id)}
            className={cn(
              "inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-[12px] transition-colors",
              on ? "border-primary bg-primary/10 font-semibold text-primary" : "border-border text-muted-foreground hover:text-foreground",
            )}
          >
            {busy === g.id && <Loader2 className="size-3 animate-spin" />}
            {g.label}
          </button>
        );
      })}
      {have.size === 0 && <span className="self-center text-[11.5px] text-muted-foreground">works its tasks only</span>}
    </div>
  );
}

/** The teams a cook is in: they decide which pots it sees. Owners of a team (and admins) can move the cook in and out. */
function CookTeams({ a }: { a: AgentRow }) {
  const me = useSession();
  const call = useV1Call();
  const teams = useV1<TeamRow[]>("/teams");
  const [busy, setBusy] = useState<string | null>(null);
  const [pick, setPick] = useState("");
  const mine = a.teams ?? [];
  const manageable = (t: string) => me.is_admin || (teams.data ?? []).some((x) => x.name === t && x.role === "owner");
  const addable = (teams.data ?? []).filter((t) => !mine.includes(t.name) && (me.is_admin || t.role === "owner"));

  async function add(team: string) {
    setBusy(team);
    try {
      await call("POST", `/teams/${team}/members`, { principal: a.name, role: "chef" });
      toast.success(`${a.name} joined ${team}`);
    } catch (e) {
      toast.error("Could not add it to that team", { description: errorMessage(e) });
    } finally {
      setBusy(null);
      setPick("");
    }
  }
  async function remove(team: string) {
    setBusy(team);
    try {
      await call("DELETE", `/teams/${team}/members/${a.name}`);
      toast.success(`${a.name} left ${team}`);
    } catch (e) {
      toast.error("Could not remove it from that team", { description: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {mine.map((t) => (
        <LabelChip key={t} className="gap-1">
          <Link to={`/teams/${t}`} className="hover:underline focus-visible:underline">{t}</Link>
          {manageable(t) && (
            <button
              type="button"
              aria-label={`Remove ${a.name} from ${t}`}
              disabled={busy === t}
              onClick={() => void remove(t)}
              className="-mr-1 inline-flex size-4 items-center justify-center rounded-full hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
            >
              {busy === t ? <Loader2 className="size-3 animate-spin" aria-hidden="true" /> : <X className="size-3" aria-hidden="true" />}
            </button>
          )}
        </LabelChip>
      ))}
      {mine.length === 0 && <span className="text-[11.5px] text-muted-foreground">in no team: it sees no pot</span>}
      {addable.length > 0 && (
        <Select value={pick} onValueChange={(v) => void add(v)} disabled={!!busy}>
          <SelectTrigger size="sm" className="h-6 w-auto gap-1 px-2 text-[12px]" aria-label={`Add ${a.name} to a team`}>
            <SelectValue placeholder="+ Add to team" />
          </SelectTrigger>
          <SelectContent>
            {addable.map((t) => (
              <SelectItem key={t.name} value={t.name}>
                {t.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
    </div>
  );
}

/** Tasks held right now out of the cook's limit, which its owner (or an admin) can step up or down. */
function LeaseLimit({ a }: { a: AgentRow }) {
  const invalidate = useInvalidateAll();
  const [busy, setBusy] = useState(false);
  async function set(n: number) {
    setBusy(true);
    try {
      await client.principals.update(a.id, { maxLeases: n });
      void invalidate();
      toast.success(`${a.name} may now hold ${n} task${n === 1 ? "" : "s"} at once`);
    } catch (e) {
      toast.error("Could not change the limit", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }
  const step = "size-6 rounded-md text-muted-foreground hover:text-foreground disabled:opacity-40";
  return (
    <span className="inline-flex items-center gap-1.5 font-mono text-[12px]">
      <span>{a.busy} of</span>
      <span className="inline-flex items-center rounded-md border border-border" role="group" aria-label={`Leases ${a.name} may hold at once`}>
        <Button type="button" variant="ghost" size="icon" className={step} disabled={busy || a.max_leases <= 1} onClick={() => void set(a.max_leases - 1)} aria-label="One fewer">
          <Minus className="size-3" />
        </Button>
        <span className="min-w-5 text-center font-bold" aria-live="polite">
          {busy ? <Loader2 className="mx-auto size-3 animate-spin" /> : a.max_leases}
        </span>
        <Button type="button" variant="ghost" size="icon" className={step} disabled={busy || a.max_leases >= 50} onClick={() => void set(a.max_leases + 1)} aria-label="One more">
          <Plus className="size-3" />
        </Button>
      </span>
    </span>
  );
}

function CookCard({
  a,
  current,
  stations,
  onToken,
}: {
  a: AgentRow;
  current: string | null;
  stations: StationView[];
  onToken: (t: { token: string; title: string; hint?: string }) => void;
}) {
  const now = useNow();
  const openTask = useOpenTask();
  const call = useV1Call();
  const [mcp, setMcp] = useState(false);
  const [confirm, setConfirm] = useState<"rotate" | "disable" | null>(null);
  const [wakeOpen, setWakeOpen] = useState(false);
  // open by default once the cook has stations; a click overrides
  const [stationsOpen, setStationsOpen] = useState<boolean | null>(null);
  const showStations = stationsOpen ?? stations.length > 0;
  const st = cookState(a, now);
  const usage = useProc("usageReport", { cook: a.name, days: 7 });

  async function rotate() {
    try {
      const r = await call<{ token: string }>("POST", `/agents/${a.name}/rotate`);
      onToken({
        token: r.token,
        title: `New token for ${a.name}`,
        hint: "The old token stopped working the moment this one was made.",
      });
    } catch (e) {
      toast.error("Could not rotate the token", { description: errorMessage(e) });
    }
  }
  async function toggle() {
    try {
      await call("POST", `/agents/${a.name}/${a.disabled ? "enable" : "disable"}`);
      toast.success(a.disabled ? `${a.name} is back in the kitchen` : `${a.name} sent home`);
    } catch (e) {
      toast.error("Could not change that", { description: errorMessage(e) });
    }
  }

  return (
    <article
      id={`cook-${a.name}`}
      aria-label={`Cook ${a.name}`}
      className={cn("scroll-mt-24 min-w-0 rounded-[14px] border border-border bg-card p-4", a.disabled && "opacity-75")}
    >
      <div className="flex items-start gap-3">
        <Avatar name={a.name} className="size-10 rounded-lg text-[18px]" />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-[18px] leading-tight">
            <Link to={`/cooks/${a.name}`} className="underline-offset-2 hover:underline focus-visible:underline">{a.name}</Link>
          </h2>
          <p className="mt-0.5 text-[12.5px] text-muted-foreground">
            {KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind} · {a.runtime ?? "local"}
            {a.owner && <> · owned by {a.owner}</>}
          </p>
        </div>
        <span
          className={cn(
            "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11.5px] font-medium",
            st === "cooking" || st === "awake" ? "border-corn/60 bg-corn/15" : "border-border bg-muted/60 text-muted-foreground",
            st === "disabled" && "border-destructive/40 text-destructive",
          )}
        >
          <span className="inline-flex">
            <StateDot state={st} className={st === "idle" ? "bg-muted-foreground/50" : ""} />
          </span>
          {STATE_LABEL[st]}
        </span>
      </div>

      <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[12.5px]">
        <dt className="text-muted-foreground">Cooking</dt>
        <dd className="min-w-0">
          {current ? (
            <button type="button" onClick={() => openTask(current)} className="font-mono text-[12px] font-bold underline-offset-2 hover:underline">
              {current}
            </button>
          ) : (
            <span className="text-muted-foreground">nothing right now</span>
          )}
        </dd>
        <dt className="text-muted-foreground">Last seen</dt>
        <dd>{fmtAgo(a.last_seen_at, now)}</dd>
        <dt className="text-muted-foreground">Leases</dt>
        <dd>
          <LeaseLimit a={a} />
        </dd>
        <dt className="pt-1 text-muted-foreground">May also</dt>
        <dd>
          <Grants a={a} />
        </dd>
        <dt className="pt-1 text-muted-foreground">Teams</dt>
        <dd className="min-w-0">
          <CookTeams a={a} />
        </dd>
      </dl>

      <div className="mt-4 flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={() => setMcp(true)}>
          <Plug /> MCP config
        </Button>
        <Button size="sm" variant="outline" onClick={() => setConfirm("rotate")}>
          <RotateCw /> Rotate token
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className={a.disabled ? "" : "text-destructive hover:text-destructive"}
          onClick={() => (a.disabled ? void toggle() : setConfirm("disable"))}
        >
          <Power /> {a.disabled ? "Enable" : "Disable"}
        </Button>
      </div>

      <button
        type="button"
        aria-expanded={showStations}
        onClick={() => setStationsOpen(!showStations)}
        className="mt-4 flex w-full items-center gap-1.5 border-t border-border pt-3 text-left"
      >
        <ChevronDown className={cn("size-3.5 text-muted-foreground transition-transform", !showStations && "-rotate-90")} aria-hidden="true" />
        <span className="label-caps">Stations</span>
        <span className="font-mono text-[11px] text-muted-foreground">{stations.length}</span>
      </button>
      {showStations && <StationList stations={stations} cook={a.name} kind={a.agent_kind} />}

      <div className="mt-4 border-t border-border pt-3">
        <span className="label-caps">Usage</span>
        <CookUsage report={usage.data} />
      </div>

      <button
        type="button"
        aria-expanded={wakeOpen}
        onClick={() => setWakeOpen((o) => !o)}
        className="mt-4 flex w-full items-center gap-1.5 border-t border-border pt-3 text-left"
      >
        <ChevronDown className={cn("size-3.5 text-muted-foreground transition-transform", !wakeOpen && "-rotate-90")} aria-hidden="true" />
        <span className="label-caps">Wake targets</span>
      </button>
      {wakeOpen && <WakeTargets agent={a.name} runtime={a.runtime} />}

      <McpDialog open={mcp} onOpenChange={setMcp} name={a.name} kind={a.agent_kind} />
      <AlertDialog open={confirm !== null} onOpenChange={(o) => !o && setConfirm(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{confirm === "rotate" ? `Rotate ${a.name}'s token?` : `Disable ${a.name}?`}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirm === "rotate"
                ? "A new token is shown once. The old one stops working immediately, so update the cook's MCP config right after."
                : "It can no longer sign in and any lease it holds will run out. You can enable it again later."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Not now</AlertDialogCancel>
            <AlertDialogAction onClick={() => void (confirm === "rotate" ? rotate() : toggle())}>
              {confirm === "rotate" ? "Rotate it" : "Disable it"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </article>
  );
}

export function WakeTargets({ agent, runtime }: { agent: string; runtime: string | null }) {
  const q = useV1<WakeTargetRow[]>(`/agents/${agent}/wake-targets`);
  const call = useV1Call();
  const me = useSession();
  const [adding, setAdding] = useState(false);
  if (q.isLoading) return <p className="mt-2 text-[12.5px] text-muted-foreground">Loading…</p>;
  const rows = q.data ?? [];
  return (
    <div className="mt-2 space-y-2">
      {rows.length === 0 && (
        <p className="text-[12.5px] text-muted-foreground">
          {runtime === "cloud"
            ? "No wake target. Cloud cooks are woken by a webhook."
            : "No wake target. A relay (fire) on your laptop wakes local cooks."}
        </p>
      )}
      {rows.map((w) => (
        <div key={w.id} className="flex items-start gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2 text-[12.5px]">
          <div className="min-w-0 flex-1">
            <p className="font-medium">{w.type}</p>
            <p className="truncate font-mono text-[11px] text-muted-foreground">
              {w.type === "webhook"
                ? String(w.config.url ?? "")
                : w.type === "claude-routine"
                  ? `Claude routine ${String(w.config.url ?? "").match(/trig_[A-Za-z0-9]+/)?.[0] ?? ""}`
                  : w.type === "github-comment"
                    ? `${String(w.config.mention ?? "the default mention")} on ${String(w.config.repo ?? "")}#${String(w.config.issue ?? "")}`
                    : "via the fire"}
            </p>
            {(w.projects.length > 0 || w.labels.length > 0) && (
              <p className="mt-0.5 text-[11px] text-muted-foreground">
                {w.projects.length > 0 && <>pots {w.projects.join(", ")} </>}
                {w.labels.length > 0 && <>labels {w.labels.join(", ")}</>}
              </p>
            )}
          </div>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label={`Remove ${w.type} wake target`}
            onClick={async () => {
              try {
                await call("DELETE", `/wake-targets/${w.id}`);
                toast.success("Wake target removed");
              } catch (e) {
                toast.error("Could not remove it", { description: errorMessage(e) });
              }
            }}
          >
            <Trash2 />
          </Button>
        </div>
      ))}
      <Button size="sm" variant="ghost" onClick={() => setAdding(true)}>
        <Plus /> Add wake target
      </Button>
      <AddWake open={adding} onOpenChange={setAdding} agent={agent} admin={me.is_admin} />
    </div>
  );
}

/**
 * Relays are added here. Cloud kinds go through a cloud setup, the same page an agent's
 * connect_cloud links to (/connect/<code>): one form and one set of rules for both.
 */
function AddWake({ open, onOpenChange, agent, admin }: { open: boolean; onOpenChange: (o: boolean) => void; agent: string; admin: boolean }) {
  const call = useV1Call();
  const proc = useProcCall();
  const navigate = useNavigate();
  const [type, setType] = useState("relay");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      if (type === "relay") {
        await call("POST", `/agents/${agent}/wake-targets`, { type, config: {} });
        toast.success("Wake target added");
        onOpenChange(false);
      } else {
        const s = await proc("cloudConnect", { kind: type, cook: agent });
        onOpenChange(false);
        navigate(`/connect/${s.code}`);
      }
    } catch (err) {
      toast.error("Could not add it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Wake target for {agent}</DialogTitle>
          <DialogDescription>How an idle cook hears that there is work, or a message for it.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="wt-type">Kind</Label>
            <Select value={type} onValueChange={setType}>
              <SelectTrigger id="wt-type" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="relay">Relay: a fire on your laptop</SelectItem>
                <SelectItem value="claude-routine">Claude Code on the web (a routine)</SelectItem>
                <SelectItem value="github-comment">GitHub comment (Codex cloud, Copilot, …)</SelectItem>
                {admin && <SelectItem value="webhook">Webhook (your own endpoint)</SelectItem>}
              </SelectContent>
            </Select>
            {type === "relay" ? (
              <p className="text-[12px] text-muted-foreground">
                The fire on {agent}&rsquo;s machine wakes its sessions (<span className="font-mono">corntchap-cli fire</span>).
              </p>
            ) : (
              <p className="text-[12px] text-muted-foreground">
                Each wake starts a fresh cloud session: new work, and every message for {agent} (pass, mentions, comments, answers), batched. Next you get
                the steps and the form, the same page {agent} gets from <span className="font-mono">connect_cloud</span>.
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />} {type === "relay" ? "Add" : "Continue"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function CreateCook({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onCreated: (c: { name: string; kind: string; token: string }) => void;
}) {
  const call = useV1Call();
  const teams = useV1<TeamRow[]>("/teams", { enabled: open });
  const [name, setName] = useState("");
  const [kind, setKind] = useState("claude-code");
  const [runtime, setRuntime] = useState("local");
  const [maxLeases, setMaxLeases] = useState("1");
  const [team, setTeam] = useState("");
  const [relay, setRelay] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setName("");
      setTeam((t) => t || teams.data?.[0]?.name || "");
    }
  }, [open, teams.data]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const made = await call<{ name: string; token: string }>("POST", "/agents", {
        name,
        kind,
        runtime,
        max_leases: Number(maxLeases) || 1,
      });
      // like `cook setup`: join the team so the cook can see its pots, and add a relay wake target for local cooks
      const notes: string[] = [];
      if (team) {
        try {
          await call("POST", `/teams/${team}/members`, { principal: made.name, role: "member" });
        } catch (err) {
          notes.push(`could not join team ${team}: ${errorMessage(err)}`);
        }
      }
      if (relay && runtime === "local") {
        try {
          await call("POST", `/agents/${made.name}/wake-targets`, { type: "relay" });
        } catch (err) {
          notes.push(`no relay wake target: ${errorMessage(err)}`);
        }
      }
      if (notes.length) toast.warning("Cook created, with a hiccup", { description: notes.join("; ") });
      onOpenChange(false);
      onCreated({ name: made.name, kind, token: made.token });
    } catch (err) {
      toast.error("Could not hire that cook", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>New cook</DialogTitle>
          <DialogDescription>Registers an agent and mints its token. The token is shown once.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="ck-name">Name</Label>
            <Input id="ck-name" required value={name} onChange={(e) => setName(e.target.value)} placeholder="claude-laptop" pattern="[A-Za-z0-9][A-Za-z0-9._-]*" autoFocus />
            <p className="text-[12px] text-muted-foreground">Letters, digits, dots, dashes. Stored in lower case.</p>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="ck-kind">Runs in</Label>
              <Select value={kind} onValueChange={setKind}>
                <SelectTrigger id="ck-kind" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Object.entries(KIND_LABEL).map(([k, v]) => (
                    <SelectItem key={k} value={k}>
                      {v}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ck-rt">Runtime</Label>
              <Select value={runtime} onValueChange={setRuntime}>
                <SelectTrigger id="ck-rt" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="cloud">Cloud</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="ck-team">Team</Label>
              <Select value={team} onValueChange={setTeam}>
                <SelectTrigger id="ck-team" className="w-full">
                  <SelectValue placeholder="No team yet" />
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
              <Label htmlFor="ck-max">Leases at once</Label>
              <Input id="ck-max" type="number" min={1} max={20} value={maxLeases} onChange={(e) => setMaxLeases(e.target.value)} />
            </div>
          </div>
          {runtime === "local" && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-muted/40 px-3 py-2.5">
              <div>
                <Label htmlFor="ck-relay">Wake through my fire</Label>
                <p className="text-[12px] text-muted-foreground">Adds a relay wake target so an idle cook is started when work arrives.</p>
              </div>
              <Switch id="ck-relay" checked={relay} onCheckedChange={setRelay} />
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !name}>
              {busy ? <Loader2 className="animate-spin" /> : <ChefHat />} Hire the cook
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
