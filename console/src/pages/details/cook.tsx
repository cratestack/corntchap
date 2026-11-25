import { useMemo, useState, type FormEvent } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useQueries } from "@tanstack/react-query";
import { Cloud, Flame, MessageSquare, Loader2, MessageCircle, Minus, Pencil, Plug, Plus, Power, RotateCw, Server, Trash2, UserPlus, Webhook } from "lucide-react";
import { toast } from "sonner";
import type { ActivityView, PrincipalView, TeamView, WakeTargetView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { PageShell } from "@/components/page";
import {
  EmptyState,
  FormSheetBody,
  FormSheetFoot,
  FormSheetHead,
  Group,
  GroupRow,
  IdChip,
  LabelChip,
  RowSkeleton,
  StatePill,
  formSheetClass,
} from "@/components/bits";
import { CloudRunList, StationList } from "@/components/stations";
import { SourceList } from "@/components/source-list";
import { VirtualMenu } from "@/components/ui/virtual-menu";
import { CookRepoScopes } from "@/components/repos";
import { UsagePanel } from "@/components/usage";
import { RestaurantLog } from "@/components/restaurant-log";
import { TaskRow } from "@/components/task-views";
import { McpDialog, TokenDialog } from "@/components/secret-dialog";
import { ConfirmDialog, DeleteDialog, RenameDialog } from "@/components/rename-delete";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { client, errorMessage } from "@/lib/api";
import { useClaimedTasks, useInvalidateAll, useProc, useProcCall, useProjects, useV1Call } from "@/lib/data";
import { useStationsSource, useTeamsSource, useWakeTargetsSource } from "@/lib/sources";
import { principalState, useCookByName, useTeamByName } from "@/lib/people";
import { useStoreValue } from "@/lib/use-store-value";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useLive } from "@/lib/live";
import { useOpenTask } from "@/lib/nav";
import { KIND_LABEL } from "@/lib/snippets";
import { canManageTeam, isSuperAdmin, useSession } from "@/lib/session";
import { Crumb, Quiet, linkClass } from "./common";

const activityLine = (a: ActivityView) => {
  const what = a.files.length > 0 ? `${a.files[0]}${a.files.length > 1 ? ` +${a.files.length - 1}` : ""}` : (a.summary ?? "");
  return `${a.tool}${what ? ` ${what}` : ""}`;
};

// ------------------------------------------------------------------ leases

/** Tasks held right now out of the cook's limit, which its owner (or a super-admin) can step up or down. */
function LeaseRow({ a }: { a: PrincipalView }) {
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
  const step = "size-10 rounded-lg border border-border bg-background pointer-coarse:size-11";
  return (
    <GroupRow
      label="Tasks at once"
      hint={
        <>
          <span className="font-mono">{a.busy}</span> held right now
        </>
      }
      trailing={
        <span className="inline-flex items-center gap-1.5" role="group" aria-label={`Leases ${a.name} may hold at once`}>
          <Button type="button" variant="ghost" size="icon" className={step} disabled={busy || a.max_leases <= 1} onClick={() => void set(a.max_leases - 1)} aria-label="One fewer">
            <Minus />
          </Button>
          <span className="min-w-7 text-center font-mono text-[16px] font-bold tabular-nums" aria-live="polite">
            {busy ? <Loader2 className="mx-auto size-4 animate-spin" /> : a.max_leases}
          </span>
          <Button type="button" variant="ghost" size="icon" className={step} disabled={busy || a.max_leases >= 50} onClick={() => void set(a.max_leases + 1)} aria-label="One more">
            <Plus />
          </Button>
        </span>
      }
    />
  );
}

// ------------------------------------------------------------------ permissions

const GRANTS: { id: string; label: string; hint: string }[] = [
  { id: "plan", label: "Plan", hint: "Edit, toss, restore and unblock any task in its teams; tick items without a lease" },
  { id: "manage", label: "Manage", hint: "Act as its owner on teams, pots, cooks, fires and wake targets" },
  { id: "admin", label: "Admin", hint: "Its owner's role powers: an orchestrator's cook acts as an orchestrator, never as a super-admin" },
];

/** What a cook may do beyond working its tasks. A cook never exceeds its owner and cannot change this itself. */
function GrantRows({ a }: { a: PrincipalView }) {
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
    <>
      {GRANTS.map((g) => (
        <label key={g.id} htmlFor={`grant-${g.id}`} className="flex min-h-14 cursor-pointer items-center gap-3 px-4 py-2.5 hover:bg-muted/50 active:bg-muted">
          <span className="min-w-0 flex-1">
            <span className="block text-[14px] font-medium leading-tight">{g.label}</span>
            <span className="mt-0.5 block text-[12px] leading-snug text-muted-foreground">{g.hint}</span>
          </span>
          {busy === g.id && <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />}
          <Switch
            id={`grant-${g.id}`}
            checked={have.has(g.id)}
            disabled={!!busy}
            onCheckedChange={() => void toggle(g.id)}
            aria-label={`${g.label}: ${g.hint}`}
            className="pointer-coarse:h-6 pointer-coarse:w-11"
          />
        </label>
      ))}
    </>
  );
}

// ------------------------------------------------------------------ teams

const teamKey = (t: TeamView) => t.name;

/** One team the cook is in: its pots, and a button to take the cook out when the signed-in person may. */
function CookTeamRow({ team, a, pots, onLeave }: { team: string; a: PrincipalView; pots: { key: string; team?: string | null }[]; onLeave: (team: string) => void }) {
  const me = useSession();
  const found = useTeamByName(team);
  // a team's owners and super-admins may move any cook in or out of it, but only its owner (or a super-admin) manages the cook itself; any chef may add or remove a cook they own
  const manageable = isSuperAdmin(me) || a.owner === me.name || canManageTeam(me, found.item?.role);
  const tp = pots.filter((p) => p.team === team);
  return (
    <GroupRow
      label={
        <Link to={`/teams/${team}`} className={linkClass}>
          {team}
        </Link>
      }
      hint={<span className="font-mono">{tp.length === 0 ? "no pots" : tp.map((p) => p.key).join(" · ")}</span>}
      trailing={
        manageable ? (
          <Button type="button" size="icon" variant="ghost" aria-label={`Remove ${a.name} from ${team}`} onClick={() => onLeave(team)}>
            <Minus />
          </Button>
        ) : undefined
      }
    />
  );
}

/** The teams a cook is in: they decide which pots it sees. Its owner, a team's owners and super-admins can move the cook in and out. */
function TeamGroup({ a, pots }: { a: PrincipalView; pots: { key: string; team?: string | null }[] }) {
  const me = useSession();
  const call = useV1Call();
  const teamQ = useStoreValue("");
  const [busy, setBusy] = useState(false);
  const [leaving, setLeaving] = useState<string | null>(null);
  const mine = a.teams;
  const addable = isSuperAdmin(me) || a.owner === me.name;
  const teams = useTeamsSource(teamQ.value, { enabled: addable });
  async function add(team: string) {
    setBusy(true);
    try {
      await call("POST", `/teams/${team}/members`, { principal: a.name, role: "chef" });
      toast.success(`${a.name} joined ${team}`);
    } catch (e) {
      toast.error("Could not add it to that team", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Group title="Teams" hint="The teams decide which pots this cook sees. With no team it sees none.">
      {mine.length === 0 && <GroupRow label="In no team" hint="It sees no pot yet." />}
      {mine.map((t) => (
        <CookTeamRow key={t} team={t} a={a} pots={pots} onLeave={setLeaving} />
      ))}
      {addable && (
        <VirtualMenu<TeamView>
          label={`Add ${a.name} to a team`}
          source={teams}
          onQuery={teamQ.set}
          itemKey={teamKey}
          mode="action"
          isDisabled={(t) => mine.includes(t.name) || busy}
          onSelect={(t) => void add(t.name)}
          empty="Every team you are in already has it."
          renderItem={(t) => (
            <span className="block min-w-0">
              <span className="block truncate text-[14px] font-medium">{t.name}</span>
              <span className="block truncate font-mono text-[11px] text-muted-foreground">
                {mine.includes(t.name) ? "already in" : `${t.members} ${t.members === 1 ? "chef" : "chefs"} · pots ${t.projects.length ? t.projects.join(", ") : "none"}`}
              </span>
            </span>
          )}
          trigger={
            <button type="button" className="flex min-h-12 w-full items-center gap-3 px-4 py-2.5 text-left outline-none transition-colors hover:bg-muted/50 focus-visible:bg-muted/60 active:bg-muted">
              <span aria-hidden="true" className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground [&_svg]:size-4">
                <UserPlus />
              </span>
              <span className="min-w-0 flex-1 text-[14px] font-medium">Add to a team</span>
            </button>
          }
        />
      )}
      <ConfirmDialog
        open={!!leaving}
        onOpenChange={(o) => !o && setLeaving(null)}
        title={`Remove ${a.name} from ${leaving ?? "the team"}?`}
        description="It stops seeing that team's pots, and any task it holds there keeps its lease until it runs out."
        action="Remove it"
        destructive
        onConfirm={async () => {
          await call("DELETE", `/teams/${leaving}/members/${a.name}`);
          toast.success(`${a.name} left ${leaving}`);
        }}
      />
    </Group>
  );
}

// ------------------------------------------------------------------ wake targets

const WAKE_KIND: Record<string, { title: string; icon: typeof Flame }> = {
  relay: { title: "Relay (a fire)", icon: Flame },
  "claude-routine": { title: "Claude Code routine", icon: Cloud },
  "github-comment": { title: "GitHub comment", icon: MessageSquare },
  webhook: { title: "Webhook", icon: Webhook },
  coder: { title: "Coder agent", icon: Server },
  devin: { title: "Devin session", icon: Server },
};

/** Where a wake target goes: the server names it (a webhook's URL, a repository and issue; secrets are never shown). */
function wakeDetail(w: WakeTargetView): string {
  if (w.type === "relay") return "wakes the sessions on its machine";
  return w.target ?? "";
}

const wakeKey = (w: WakeTargetView) => w.id;

function WakeGroup({ agent, runtime }: { agent: string; runtime: string | null }) {
  const targets = useWakeTargetsSource(agent);
  const call = useV1Call();
  const me = useSession();
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<WakeTargetView | null>(null);
  const cloud = runtime === "cloud";
  return (
    <Group
      title="Wake targets"
      hint={
        cloud
          ? "A cook with cloud targets only is fire and forget: ready work in its pots (and its owner's direct messages) start a run; it opens a pull request and stops."
          : "How an idle cook hears there is work: a fire on its machine wakes its sessions."
      }
    >
      <SourceList<WakeTargetView>
        source={targets}
        more="button"
        itemKey={wakeKey}
        label="Wake targets"
        empty={
          <>
            <p className="font-medium text-foreground">No wake target</p>
            <p>{cloud ? "Nothing can start it yet. Connect a routine, GitHub comments or a webhook." : "Add a relay so a fire wakes it."}</p>
          </>
        }
        renderItem={(w) => {
          const kind = WAKE_KIND[w.type] ?? { title: w.type, icon: Cloud };
          const Icon = kind.icon;
          return (
            <GroupRow
              icon={<Icon />}
              label={<span className="flex flex-wrap items-center gap-x-2 gap-y-1">{kind.title}</span>}
              hint={
                <>
                  <span className="block truncate font-mono text-[11px]">{wakeDetail(w)}</span>
                  {w.session_url && (
                    <a href={w.session_url} target="_blank" rel="noreferrer" className="block truncate text-[11.5px] underline underline-offset-2">
                      Latest session
                    </a>
                  )}
                  {(w.projects.length > 0 || w.labels.length > 0) && (
                    <span className="block text-[11.5px]">
                      {w.projects.length > 0 && <>pots {w.projects.join(", ")} </>}
                      {w.labels.length > 0 && <>labels {w.labels.join(", ")}</>}
                    </span>
                  )}
                </>
              }
              trailing={
                <Button type="button" size="icon" variant="ghost" aria-label={`Remove ${kind.title} wake target`} onClick={() => setRemoving(w)}>
                  <Trash2 />
                </Button>
              }
            />
          );
        }}
      />
      <GroupRow icon={<Plus />} label="Add a wake target" hint={cloud ? "Goes to the cloud setup steps" : undefined} onClick={() => setAdding(true)} />
      <AddWake open={adding} onOpenChange={setAdding} agent={agent} runtime={runtime} admin={isSuperAdmin(me)} />
      <ConfirmDialog
        open={!!removing}
        onOpenChange={(o) => !o && setRemoving(null)}
        title="Remove this wake target?"
        description={`${agent} will no longer be started this way. You can add it again.`}
        action="Remove it"
        destructive
        onConfirm={async () => {
          await call("DELETE", `/wake-targets/${removing!.id}`);
          toast.success("Wake target removed");
        }}
      />
    </Group>
  );
}

/**
 * Relays are added here. Cloud kinds go through a cloud setup, the same page an agent's
 * connect_cloud links to (/connect/<code>): one form and one set of rules for both.
 */
export function AddWake({
  open,
  onOpenChange,
  agent,
  runtime,
  admin,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  agent: string;
  runtime: string | null;
  admin: boolean;
}) {
  const call = useV1Call();
  const proc = useProcCall();
  const navigate = useNavigate();
  const [type, setType] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const choice = type ?? (runtime === "cloud" ? "claude-routine" : "relay");
  const options = [
    { id: "relay", title: "Relay", sub: "A fire on your laptop wakes its sessions", icon: Flame },
    { id: "claude-routine", title: "Claude Code on the web", sub: "A routine is started on each wake", icon: Cloud },
    { id: "github-comment", title: "GitHub comment", sub: "Codex cloud, Copilot and others", icon: MessageSquare },
    { id: "devin", title: "Devin", sub: "A Devin session on your Devin organization", icon: Server },
    ...(admin
      ? [
          { id: "webhook", title: "Webhook", sub: "Your own endpoint", icon: Webhook },
          { id: "coder", title: "Coder", sub: "An agent run on your Coder deployment", icon: Server },
        ]
      : []),
  ];
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      if (choice === "relay") {
        await call("POST", `/agents/${agent}/wake-targets`, { type: choice, config: {} });
        toast.success("Wake target added");
        onOpenChange(false);
      } else {
        const s = await proc("cloudConnect", { kind: choice, cook: agent });
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
      <DialogContent className={cn(formSheetClass, "sm:max-w-md")}>
        <form onSubmit={(e) => void submit(e)} className="flex min-h-0 flex-1 flex-col">
          <FormSheetHead>
            <DialogTitle className="text-[22px] leading-tight">Wake target for {agent}</DialogTitle>
            <DialogDescription>How an idle cook hears that there is work, or a message for it.</DialogDescription>
          </FormSheetHead>
          <FormSheetBody>
            <div role="radiogroup" aria-label="Kind" className="divide-y divide-border overflow-hidden rounded-xl border border-border">
              {options.map((o) => {
                const on = choice === o.id;
                return (
                  <button
                    key={o.id}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    onClick={() => setType(o.id)}
                    className={cn("flex min-h-14 w-full items-center gap-3 px-3.5 py-2.5 text-left transition-colors", on ? "bg-primary/5" : "hover:bg-muted/50")}
                  >
                    <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
                      <o.icon className="size-4" aria-hidden="true" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block text-[14px] font-medium">{o.title}</span>
                      <span className="block text-[12px] text-muted-foreground">{o.sub}</span>
                    </span>
                    <span className={cn("grid size-5 shrink-0 place-items-center rounded-full border", on ? "border-primary bg-primary" : "border-border")}>
                      {on && <span className="size-2 rounded-full bg-primary-foreground" />}
                    </span>
                  </button>
                );
              })}
            </div>
            <p className="pb-2 text-[12px] text-muted-foreground">
              {choice === "relay" ? (
                <>
                  The fire on {agent}&rsquo;s machine wakes its sessions (<span className="font-mono">corntchap-cli fire</span>).
                </>
              ) : (
                <>
                  Each wake starts a fresh cloud session. Next you get the steps and the form, the same page {agent} gets from <span className="font-mono">connect_cloud</span>.
                </>
              )}
            </p>
          </FormSheetBody>
          <FormSheetFoot>
            <Button type="submit" disabled={busy} className="max-sm:h-11">
              {busy && <Loader2 className="animate-spin" />} {choice === "relay" ? "Add the relay" : "Continue to setup"}
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

// ------------------------------------------------------------------ the page

const RECENT = 15;

type Dlg = "rename" | "delete" | "rotate" | "disable" | "mcp" | null;

export function CookDetailsPage() {
  const { name = "" } = useParams();
  const found = useCookByName(name);
  const projects = useProjects();
  const sessions = useStationsSource({ cook: name, kind: "session" });
  const cloudRuns = useStationsSource({ cook: name, kind: "cloud-run" });
  const usage = useProc("usageReport", { cook: name, days: 7 });
  const claimed = useClaimedTasks(projects.data);
  const { events } = useLive();
  const openTask = useOpenTask();
  const navigate = useNavigate();
  const call = useV1Call();
  const proc = useProcCall();
  const now = useNow();
  const [dialog, setDialog] = useState<Dlg>(null);
  const [token, setToken] = useState<{ token: string; title: string; hint?: string } | null>(null);

  const cook = found.item;

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

  // what the cook's stations reported last, a page for each pot of its teams (the latest few: `activityPage` has the rest)
  const stationPots = useMemo(() => pots.map((p) => p.key), [pots]);
  const activity = useQueries({
    queries: stationPots.map((project) => ({
      queryKey: ["data", "activity", project, "cook", name],
      queryFn: () => client.procedures.activityPage({ project, cook: name, page: { limit: RECENT, cursor: null } }),
      staleTime: 1500,
      refetchInterval: 15000,
    })),
  });
  const recent = useMemo(() => {
    const rows: (ActivityView & { project: string })[] = [];
    activity.forEach((r, i) => {
      for (const a of r.data?.items ?? []) rows.push({ ...a, project: stationPots[i]! });
    });
    return rows.sort((a, b) => b.at.localeCompare(a.at)).slice(0, RECENT);
  }, [activity, stationPots]);

  const crumb = <Crumb to="/cooks" parent="Cooks" here={name} />;

  if (found.isLoading) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Cook" title={name} className="pt-2">
          <RowSkeleton rows={3} />
        </PageShell>
      </>
    );
  }
  if (found.error || !cook) {
    return (
      <>
        {crumb}
        <PageShell eyebrow="Cook" title={name} className="pt-2">
          <EmptyState title={found.error ? "Could not reach the cooks" : "No such cook"} action={<Button asChild variant="outline"><Link to="/cooks">Back to Cooks</Link></Button>}>
            {found.error ? errorMessage(found.error) : `${name} is not a cook you can see.`}
          </EmptyState>
        </PageShell>
      </>
    );
  }

  const st = principalState(cook, now);
  const current = held[0];

  async function rotate() {
    const r = await call<{ token: string }>("POST", `/agents/${cook!.name}/rotate`);
    setToken({ token: r.token, title: `New token for ${cook!.name}`, hint: "The old token stopped working the moment this one was made." });
  }
  async function setEnabled(enable: boolean) {
    try {
      await call("POST", `/agents/${cook!.name}/${enable ? "enable" : "disable"}`);
      toast.success(enable ? `${cook!.name} is back in the kitchen` : `${cook!.name} sent home`);
    } catch (e) {
      toast.error("Could not change that", { description: errorMessage(e) });
    }
  }

  return (
    <>
      {crumb}
      <PageShell
        eyebrow="Cook"
        title={cook.name}
        className="pt-2 sm:pt-3"
        sub={
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
            <StatePill state={st} />
            <span>
              {KIND_LABEL[cook.agent_kind ?? "other"] ?? cook.agent_kind ?? "Agent"} · {cook.runtime ?? "local"}
              {cook.owner && <> · owned by {cook.owner}</>}
            </span>
          </span>
        }
      >
        <div className="space-y-6 lg:grid lg:grid-cols-2 lg:items-start lg:gap-x-6 lg:space-y-0 lg:gap-y-6">
          <div className="space-y-6">
            <Group title="Overview">
              <GroupRow
                label="Cooking"
                value={current ? <span className="font-mono font-bold text-foreground">{current.id}</span> : "nothing right now"}
                {...(current ? { onClick: () => openTask(current.id) } : {})}
              />
              <GroupRow label="Last seen" value={fmtAgo(cook.last_seen_at ?? null, now)} />
              <GroupRow label="Runs in" value={`${KIND_LABEL[cook.agent_kind ?? "other"] ?? cook.agent_kind ?? "other"} · ${cook.runtime ?? "local"}`} />
              <GroupRow label="Owner" value={cook.owner ?? "none"} />
            </Group>

            <Group title="Leases">
              <LeaseRow a={cook} />
            </Group>

            <Group title="Permissions" hint="What it may do beyond working its tasks. A cook never exceeds its owner and cannot change this itself.">
              <GrantRows a={cook} />
            </Group>

            <TeamGroup a={cook} pots={pots} />

            <CookRepoScopes cook={cook.name} pots={pots} scopes={cook.repo_scopes} />

            <Group title="Connect" hint="The token is shown once. Rotating makes a new one and the old one stops working at once.">
              <GroupRow icon={<Plug />} label="MCP config" hint="Snippets for OpenCode, Claude Code, Codex, Copilot and Antigravity" onClick={() => setDialog("mcp")} />
              <GroupRow icon={<RotateCw />} label="Rotate token" hint="Make a new token and see the config with it filled in" onClick={() => setDialog("rotate")} />
            </Group>
          </div>

          <div className="space-y-6">
            <Group title="Sessions" aside={<span className="font-mono text-[11px] text-muted-foreground">{sessions.items.length}{sessions.hasNextPage ? "+" : ""}</span>}>
              <div className="p-3.5">
                {sessions.isLoading ? (
                  <RowSkeleton rows={2} />
                ) : (
                  <>
                    <StationList source={sessions} cook={cook.name} kind={cook.agent_kind ?? null} more="button" />
                  </>
                )}
              </div>
            </Group>

            {cloudRuns.items.length > 0 && (
              <Group
                title="Cloud runs"
                hint="One station per run: what a run claims, comments and passes is attributed to it."
                aside={<span className="font-mono text-[11px] text-muted-foreground">{cloudRuns.items.length}{cloudRuns.hasNextPage ? "+" : ""}</span>}
              >
                <div className="p-3.5">
                  <CloudRunList source={cloudRuns} more="button" />
                </div>
              </Group>
            )}

            <WakeGroup agent={cook.name} runtime={cook.runtime ?? null} />

            <RestaurantLog cook={cook.name} />

            <Group title="Usage" hint="Estimated tokens corntchap sent this cook.">
              <div className="p-3.5">
                <UsagePanel report={usage.data} />
              </div>
            </Group>

            <Group title="Tasks" hint="Held right now, and served lately.">
              <div className="space-y-2 p-3">
                {claimed.isLoading ? (
                  <RowSkeleton rows={2} />
                ) : held.length === 0 && served.length === 0 ? (
                  <Quiet>Nothing held and nothing served in this session's feed.</Quiet>
                ) : (
                  <>
                    {held.map((t) => (
                      <TaskRow key={t.id} task={t} selected={false} onOpen={openTask} />
                    ))}
                    {served.length > 0 && (
                      <>
                        <p className="label-caps pt-2">Recently served</p>
                        <ul className="divide-y divide-border">
                          {served.map((s) => (
                            <li key={`${s.task}${s.at}`} className="flex min-h-11 items-center justify-between gap-3">
                              <button type="button" onClick={() => openTask(s.task)} className={`font-mono text-[12px] font-bold ${linkClass}`}>
                                {s.task}
                              </button>
                              <span className="font-mono text-[10.5px] text-muted-foreground">{fmtAgo(s.at, now)}</span>
                            </li>
                          ))}
                        </ul>
                      </>
                    )}
                  </>
                )}
              </div>
            </Group>

            <Group title="Recent activity" hint={`What its stations' hooks reported last (the latest ${RECENT}).`}>
              <div className="p-3.5">
                {stationPots.length === 0 ? (
                  <Quiet>The cook is in no pot, so no activity is reported.</Quiet>
                ) : activity.some((r) => r.isLoading) && recent.length === 0 ? (
                  <RowSkeleton rows={2} />
                ) : recent.length === 0 ? (
                  <Quiet>Nothing reported yet.</Quiet>
                ) : (
                  <ul className="space-y-2">
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
              </div>
            </Group>

            <Group title="Pass" hint={`${plural(pots.length, "pot")} visible to ${cook.name}.`}>
              {pots.length === 0 && <GroupRow label="No pots, so no conversations" />}
              {pots.map((p) => (
                <GroupRow
                  key={p.key}
                  icon={<MessageCircle />}
                  label={`Talk with ${cook.name}`}
                  value={<span className="font-mono font-bold">{p.key}</span>}
                  to={`/pots/${p.key}/pass?with=${encodeURIComponent(cook.name)}`}
                />
              ))}
            </Group>
          </div>

          <Group title="Danger zone" tone="danger" className="lg:col-span-2 lg:max-w-xl">
            {cook.disabled ? (
              <GroupRow icon={<Power />} label="Enable" hint="Let it sign in and work again" onClick={() => void setEnabled(true)} />
            ) : (
              <GroupRow icon={<Power />} label="Disable" hint="It can no longer sign in; any lease it holds runs out" destructive onClick={() => setDialog("disable")} />
            )}
            <GroupRow icon={<Pencil />} label="Rename" hint="Tokens, stations, leases and teams keep working" onClick={() => setDialog("rename")} />
            <GroupRow icon={<Trash2 />} label="Delete" hint="Its tokens, stations and wake targets go. This cannot be undone" destructive onClick={() => setDialog("delete")} />
          </Group>
        </div>
      </PageShell>

      <McpDialog open={dialog === "mcp"} onOpenChange={(o) => !o && setDialog(null)} name={cook.name} kind={cook.agent_kind ?? null} />
      <TokenDialog
        open={!!token}
        onOpenChange={(o) => !o && setToken(null)}
        title={token?.title ?? ""}
        name={cook.name}
        token={token?.token ?? null}
        kind={cook.agent_kind ?? null}
        {...(token?.hint ? { hint: token.hint } : {})}
      />
      <ConfirmDialog
        open={dialog === "rotate"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Rotate ${cook.name}'s token?`}
        description="A new token is shown once. The old one stops working immediately, so update the cook's MCP config right after."
        action="Rotate it"
        onConfirm={async () => {
          try {
            await rotate();
          } catch (e) {
            toast.error("Could not rotate the token", { description: errorMessage(e) });
          }
        }}
      />
      <ConfirmDialog
        open={dialog === "disable"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Disable ${cook.name}?`}
        description="It can no longer sign in and any lease it holds will run out. You can enable it again later."
        action="Disable it"
        destructive
        onConfirm={() => setEnabled(false)}
      />
      <RenameDialog
        open={dialog === "rename"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Rename ${cook.name}`}
        description="Its tokens, stations, leases, wake targets and teams keep working. Text that already says the old name (@mentions, and the agent name in a fire's config.toml) does not change: edit the config on the machine."
        label="New name"
        current={cook.name}
        onSave={async (newName) => {
          await proc("cookRename", { name: cook.name, newName });
          toast.success(`${cook.name} is now ${newName.toLowerCase()}`);
          void navigate(`/cooks/${newName.toLowerCase()}`, { replace: true });
        }}
      />
      <DeleteDialog
        open={dialog === "delete"}
        onOpenChange={(o) => !o && setDialog(null)}
        title={`Delete ${cook.name}?`}
        description="Any task it holds goes back to open. Its tokens, stations, wake targets and team memberships are deleted, so it can no longer sign in. Comments and tasks it wrote stay, without an author. This cannot be undone."
        action="Delete the cook"
        onConfirm={async () => {
          const out = await proc("cookDelete", { name: cook.name });
          toast.success(`${cook.name} deleted`, { description: out.released > 0 ? `${out.released} lease${out.released === 1 ? "" : "s"} released.` : undefined });
          void navigate("/cooks", { replace: true });
        }}
      />
    </>
  );
}
