import { useEffect, useMemo, useState } from "react";
import { Link, useLocation, useNavigate, useSearchParams } from "react-router";
import { ChefHat, ChevronRight, Cloud, Flame, Laptop, Loader2, Minus, Plus, MessageSquare, Webhook } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { PageShell } from "@/components/page";
import {
  Avatar,
  EmptyState,
  FilterChips,
  FormSheetBody,
  FormSheetFoot,
  FormSheetHead,
  LabelChip,
  RowSkeleton,
  StatePill,
  StateDot,
  STATE_LABEL,
  cookState,
  formSheetClass,
  type CookState,
} from "@/components/bits";
import { fmtTokens } from "@/components/usage";
import { Snippets, TokenBox } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useAgents, useClaimedTasks, useProc, useProcCall, useProjects, useV1, useV1Call, type AgentRow, type TeamRow } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { KIND_LABEL } from "@/lib/snippets";
import { useSession } from "@/lib/session";

type Filter = "all" | "awake" | "cloud" | "local" | "disabled";

const STATE_ORDER: Record<CookState, number> = { cooking: 0, awake: 1, idle: 2, disabled: 3 };

export function CooksPage() {
  const agents = useAgents();
  const projects = useProjects();
  const claimed = useClaimedTasks(projects.data);
  const usage = useProc("usageReport", { days: 7 });
  const [creating, setCreating] = useState(false);
  const [params, setParams] = useSearchParams();
  const loc = useLocation();
  const navigate = useNavigate();
  const now = useNow();

  const filter = (["all", "awake", "cloud", "local", "disabled"] as const).find((f) => f === params.get("show")) ?? "all";
  const setFilter = (f: Filter) => setParams(f === "all" ? {} : { show: f }, { replace: true });

  // old links to /cooks#name now land on the cook's page
  useEffect(() => {
    if (loc.hash && agents.data?.some((a) => a.name === loc.hash.slice(1))) void navigate(`/cooks/${loc.hash.slice(1)}`, { replace: true });
  }, [loc.hash, agents.data, navigate]);

  const claimedTasks = claimed.tasks;
  const current = useMemo(() => {
    const m = new Map<string, (typeof claimedTasks)[number]>();
    for (const t of claimedTasks) if (t.claimed_by && t.lease_live) m.set(t.claimed_by, t);
    return m;
  }, [claimedTasks]);

  const all = useMemo(
    () =>
      [...(agents.data ?? [])].sort(
        (a, b) => STATE_ORDER[cookState(a, now)] - STATE_ORDER[cookState(b, now)] || a.name.localeCompare(b.name),
      ),
    [agents.data, now],
  );
  const counts = useMemo(() => {
    const awake = (a: AgentRow) => ["cooking", "awake"].includes(cookState(a, now));
    return {
      all: all.length,
      awake: all.filter(awake).length,
      cloud: all.filter((a) => a.runtime === "cloud").length,
      local: all.filter((a) => a.runtime !== "cloud").length,
      disabled: all.filter((a) => a.disabled).length,
    };
  }, [all, now]);
  const shown = all.filter((a) => {
    const st = cookState(a, now);
    switch (filter) {
      case "awake":
        return st === "cooking" || st === "awake";
      case "cloud":
        return a.runtime === "cloud";
      case "local":
        return a.runtime !== "cloud";
      case "disabled":
        return a.disabled;
      default:
        return true;
    }
  });

  return (
    <PageShell
      eyebrow="Kitchen"
      title="Cooks"
      sub={
        <>
          The agents that scoop and serve your tasks.
          {usage.data && (
            <span className="mt-1 block" title={usage.data.note}>
              Tokens sent, 7 days: <span className="font-mono font-bold">~{fmtTokens(usage.data.totals.tokens)}</span> (an estimate)
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
        <RowSkeleton rows={4} />
      ) : agents.error ? (
        <EmptyState band={false} title="Could not reach the cooks">{errorMessage(agents.error)}</EmptyState>
      ) : all.length === 0 ? (
        <EmptyState title="No cooks yet" action={<Button onClick={() => setCreating(true)}><Plus /> Hire the first cook</Button>}>
          A cook is an agent (OpenCode, Claude Code, Codex, Copilot, Antigravity or any MCP client). Make one, copy its token or MCP config, and it can start scooping.
        </EmptyState>
      ) : (
        <>
          <FilterChips
            label="Show cooks"
            value={filter}
            onChange={setFilter}
            className="mb-4"
            options={[
              { id: "all", label: "All", count: counts.all },
              { id: "awake", label: "Awake", count: counts.awake },
              { id: "cloud", label: "Cloud", count: counts.cloud },
              { id: "local", label: "Local", count: counts.local },
              ...(counts.disabled > 0 || filter === "disabled" ? [{ id: "disabled" as const, label: "Disabled", count: counts.disabled }] : []),
            ]}
          />
          {shown.length === 0 ? (
            <EmptyState band={false} title="No cook matches" action={<Button variant="outline" onClick={() => setFilter("all")}>Show all cooks</Button>}>
              Nothing is {filter === "awake" ? "awake" : filter} right now.
            </EmptyState>
          ) : (
            <ul
              className={cn(
                "max-md:divide-y max-md:divide-border max-md:overflow-hidden max-md:rounded-[14px] max-md:border max-md:border-border max-md:bg-card",
                "md:grid md:grid-cols-2 md:gap-3 2xl:grid-cols-3",
              )}
            >
              {shown.map((a) => (
                <CookItem key={a.name} a={a} current={current.get(a.name)?.id ?? null} now={now} />
              ))}
            </ul>
          )}
        </>
      )}
      <NewCookSheet open={creating} onOpenChange={setCreating} />
    </PageShell>
  );
}

/** Used leases as little blocks: saffron while held. */
function LeaseBlocks({ busy, max }: { busy: number; max: number }) {
  const n = Math.min(max, 10);
  return (
    <span className="inline-flex items-center gap-[3px]" aria-hidden="true">
      {Array.from({ length: n }).map((_, i) => (
        <span key={i} className={cn("h-2 w-3 rounded-[2px]", i < busy ? "bg-corn" : "bg-muted")} />
      ))}
    </span>
  );
}

/** One cook: a two-line row on phones, a compact card from tablets up. The whole thing opens the cook's page. */
function CookItem({ a, current, now }: { a: AgentRow; current: string | null; now: number }) {
  const st = cookState(a, now);
  const cloud = a.runtime === "cloud";
  const kind = KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind ?? "Agent";
  return (
    <li className="min-w-0 md:contents">
      <Link
        to={`/cooks/${a.name}`}
        aria-label={`Cook ${a.name}, ${STATE_LABEL[st]}`}
        className={cn(
          "group flex min-h-16 items-center gap-3 px-4 py-3 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted",
          "md:block md:min-h-0 md:rounded-[14px] md:border md:border-border md:bg-card md:p-4 md:hover:border-foreground/30 md:focus-visible:border-foreground/40",
          a.disabled && "opacity-75",
        )}
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <Avatar name={a.name} className="size-10 rounded-lg text-[18px]" />
          <div className="min-w-0 flex-1">
            <p className="flex items-center gap-2">
              <span className="truncate font-display text-[17px] leading-tight">{a.name}</span>
              {cloud && (
                <LabelChip className="gap-1 px-1.5">
                  <Cloud className="size-3" aria-hidden="true" /> cloud
                </LabelChip>
              )}
            </p>
            <p className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
              {kind} · {a.runtime ?? "local"}
            </p>
            <p className="mt-0.5 flex items-center gap-1.5 text-[12.5px] md:hidden">
              <StateDot state={st} className={st === "idle" ? "bg-muted-foreground/50" : ""} />
              <span className={cn(st === "disabled" ? "text-destructive" : st === "idle" ? "text-muted-foreground" : "text-foreground")}>{STATE_LABEL[st]}</span>
              {current && <span className="font-mono text-[12px] font-bold">{current}</span>}
            </p>
          </div>
          <StatePill state={st} className="max-md:hidden" />
          <div className="flex shrink-0 items-center gap-1.5 md:hidden">
            <span className="text-right font-mono text-[12.5px] leading-tight">
              <span className="font-bold">{a.busy}</span>
              <span className="text-muted-foreground">/{a.max_leases}</span>
              <span className="block text-[10px] uppercase tracking-wider text-muted-foreground">leases</span>
            </span>
            <ChevronRight className="size-4 text-muted-foreground/70" aria-hidden="true" />
          </div>
        </div>
        <dl className="mt-4 grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-[12.5px] max-md:hidden">
          <dt className="text-muted-foreground">Cooking</dt>
          <dd className="min-w-0 truncate">
            {current ? <span className="font-mono text-[12px] font-bold">{current}</span> : <span className="text-muted-foreground">nothing right now</span>}
          </dd>
          <dt className="text-muted-foreground">Leases</dt>
          <dd className="flex items-center gap-2 font-mono text-[12px]">
            <span>
              <b>{a.busy}</b>/{a.max_leases}
            </span>
            <LeaseBlocks busy={a.busy} max={a.max_leases} />
          </dd>
          <dt className="text-muted-foreground">Last seen</dt>
          <dd>{fmtAgo(a.last_seen_at, now)}</dd>
          <dt className="text-muted-foreground">Teams</dt>
          <dd className="flex min-w-0 flex-wrap gap-1">
            {(a.teams ?? []).length === 0 ? <span className="text-muted-foreground">none</span> : (a.teams ?? []).map((t) => <LabelChip key={t}>{t}</LabelChip>)}
          </dd>
        </dl>
      </Link>
    </li>
  );
}

// ------------------------------------------------------------------ New cook: a step-by-step sheet

type Step = "who" | "where" | "token" | "wake";

const CLOUD_WAKES = [
  { id: "claude-routine", title: "Claude Code on the web", sub: "A routine is started on each wake", icon: Cloud },
  { id: "github-comment", title: "GitHub comment", sub: "Codex cloud, Copilot and others", icon: MessageSquare },
  { id: "webhook", title: "Webhook", sub: "Your own endpoint (admins)", icon: Webhook },
] as const;

/**
 * create, then the token and MCP config, with the team and a relay wake target added on the way, then (for a cloud
 * cook) the cloud setup. On a phone a bottom sheet with one thing per step and a sticky button.
 */
function NewCookSheet({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const call = useV1Call();
  const proc = useProcCall();
  const navigate = useNavigate();
  const me = useSession();
  const teams = useV1<TeamRow[]>("/teams", { enabled: open });
  const [step, setStep] = useState<Step>("who");
  const [name, setName] = useState("");
  const [kind, setKind] = useState("claude-code");
  const [runtime, setRuntime] = useState("local");
  const [maxLeases, setMaxLeases] = useState(1);
  const [team, setTeam] = useState("");
  const [relay, setRelay] = useState(true);
  const [busy, setBusy] = useState(false);
  const [made, setMade] = useState<{ name: string; token: string } | null>(null);

  const [wasOpen, setWasOpen] = useState(false);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setStep("who");
      setName("");
      setMade(null);
      setMaxLeases(1);
    }
  }
  useEffect(() => {
    if (open) setTeam((t) => t || (teams.data?.find((x) => x.role === "owner")?.name ?? teams.data?.[0]?.name) || "");
  }, [open, teams.data, me.is_admin]);

  const cloud = runtime === "cloud";
  const total = cloud ? 4 : 3;
  const index = ["who", "where", "token", "wake"].indexOf(step) + 1;
  const nameOk = /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name);

  async function hire() {
    setBusy(true);
    try {
      const m = await call<{ name: string; token: string }>("POST", "/agents", { name, kind, runtime, max_leases: maxLeases });
      // like `cook setup`: join the team so the cook can see its pots, and add a relay wake target for local cooks
      const notes: string[] = [];
      if (team) {
        try {
          await call("POST", `/teams/${team}/members`, { principal: m.name, role: "member" });
        } catch (err) {
          notes.push(`could not join team ${team}: ${errorMessage(err)}`);
        }
      }
      if (relay && !cloud) {
        try {
          await call("POST", `/agents/${m.name}/wake-targets`, { type: "relay" });
        } catch (err) {
          notes.push(`no relay wake target: ${errorMessage(err)}`);
        }
      }
      if (notes.length) toast.warning("Cook created, with a hiccup", { description: notes.join("; ") });
      setMade(m);
      setStep("token");
    } catch (err) {
      toast.error("Could not hire that cook", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  async function wake(k: string) {
    if (!made) return;
    setBusy(true);
    try {
      const s = await proc("cloudConnect", { kind: k, cook: made.name });
      onOpenChange(false);
      void navigate(`/connect/${s.code}`);
    } catch (err) {
      toast.error("Could not start the cloud setup", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  function finish() {
    onOpenChange(false);
    if (made) void navigate(`/cooks/${made.name}`);
  }

  const optionBtn = (on: boolean) =>
    cn(
      "flex min-h-14 w-full items-center gap-3 rounded-xl border px-3.5 py-2.5 text-left transition-colors",
      on ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50",
    );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className={cn(formSheetClass, "sm:max-w-lg")}
        onInteractOutside={(e) => step === "token" && e.preventDefault()}
        onEscapeKeyDown={(e) => step === "token" && e.preventDefault()}
      >
        <FormSheetHead>
          <p className="label-caps" aria-live="polite">
            Step {index} of {total}
          </p>
          <div className="flex gap-1.5" aria-hidden="true">
            {Array.from({ length: total }).map((_, i) => (
              <span key={i} className={cn("h-1 flex-1 rounded-full", i < index ? "bg-primary" : "bg-muted")} />
            ))}
          </div>
          <DialogTitle className="mt-1 text-[24px] leading-tight">
            {step === "who" && "Who is cooking?"}
            {step === "where" && "Where does it work?"}
            {step === "token" && `${made?.name ?? "The cook"} is in the kitchen`}
            {step === "wake" && "How is it woken?"}
          </DialogTitle>
          <DialogDescription>
            {step === "who" && "Name the agent and say where it runs. Its token is made when you hire it."}
            {step === "where" && "The team decides which pots it sees; the lease limit is how many tasks it holds at once."}
            {step === "token" && "Copy the token now: it is shown once and the server keeps only a hash. Then paste the config where the agent runs."}
            {step === "wake" && `${made?.name} runs in the cloud, so it is on call: started by a message, an @mention or a comment on its task. Pick how.`}
          </DialogDescription>
        </FormSheetHead>

        {step === "who" && (
          <form
            id="new-cook-who"
            className="flex min-h-0 flex-1 flex-col"
            onSubmit={(e) => {
              e.preventDefault();
              if (nameOk) setStep("where");
            }}
          >
            <FormSheetBody>
              <div className="space-y-1.5">
                <Label htmlFor="ck-name">Name</Label>
                <Input
                  id="ck-name"
                  required
                  autoFocus
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="claude-laptop"
                  className="h-11 font-mono"
                />
                <p className="text-[12px] text-muted-foreground">Letters, digits, dots, dashes. Stored in lower case.</p>
              </div>
              <div className="space-y-1.5">
                <Label>Runs in</Label>
                <div role="radiogroup" aria-label="Runs in" className="grid grid-cols-2 gap-2">
                  {Object.entries(KIND_LABEL).map(([k, v]) => (
                    <button key={k} type="button" role="radio" aria-checked={kind === k} onClick={() => setKind(k)} className={cn(optionBtn(kind === k), "min-h-12 text-[13.5px] font-medium")}>
                      {v}
                    </button>
                  ))}
                </div>
              </div>
              <div className="space-y-1.5 pb-2">
                <Label>Where it runs</Label>
                <div role="radiogroup" aria-label="Runtime" className="grid gap-2 sm:grid-cols-2">
                  <button type="button" role="radio" aria-checked={runtime === "local"} onClick={() => setRuntime("local")} className={optionBtn(runtime === "local")}>
                    <Laptop className="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span>
                      <span className="block text-[14px] font-medium">Local</span>
                      <span className="block text-[12px] text-muted-foreground">On your machine, woken by a fire</span>
                    </span>
                  </button>
                  <button type="button" role="radio" aria-checked={runtime === "cloud"} onClick={() => setRuntime("cloud")} className={optionBtn(runtime === "cloud")}>
                    <Cloud className="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span>
                      <span className="block text-[14px] font-medium">Cloud</span>
                      <span className="block text-[12px] text-muted-foreground">On call: started by a message or mention</span>
                    </span>
                  </button>
                </div>
              </div>
            </FormSheetBody>
            <FormSheetFoot>
              <Button type="submit" disabled={!nameOk} className="max-sm:h-11">
                Next
              </Button>
              <Button type="button" variant="ghost" className="max-sm:h-11" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
            </FormSheetFoot>
          </form>
        )}

        {step === "where" && (
          <form
            className="flex min-h-0 flex-1 flex-col"
            onSubmit={(e) => {
              e.preventDefault();
              void hire();
            }}
          >
            <FormSheetBody>
              <div className="space-y-1.5">
                <Label htmlFor="ck-team">Team</Label>
                <Select value={team} onValueChange={setTeam}>
                  <SelectTrigger id="ck-team" className="h-11 w-full">
                    <SelectValue placeholder="No team yet: it sees no pot" />
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
              <div className="flex min-h-14 items-center justify-between gap-3 rounded-xl border border-border px-3.5 py-2">
                <div>
                  <p className="text-[14px] font-medium">Tasks at once</p>
                  <p className="text-[12px] text-muted-foreground">How many leases it may hold</p>
                </div>
                <div className="flex items-center gap-1.5" role="group" aria-label="Leases at once">
                  <Button type="button" variant="ghost" size="icon" className="border border-border" aria-label="One fewer" disabled={maxLeases <= 1} onClick={() => setMaxLeases((n) => n - 1)}>
                    <Minus />
                  </Button>
                  <span className="min-w-7 text-center font-mono text-[16px] font-bold">{maxLeases}</span>
                  <Button type="button" variant="ghost" size="icon" className="border border-border" aria-label="One more" disabled={maxLeases >= 20} onClick={() => setMaxLeases((n) => n + 1)}>
                    <Plus />
                  </Button>
                </div>
              </div>
              {!cloud ? (
                <label htmlFor="ck-relay" className="flex min-h-14 cursor-pointer items-center justify-between gap-3 rounded-xl border border-border px-3.5 py-2.5">
                  <span>
                    <span className="flex items-center gap-1.5 text-[14px] font-medium">
                      <Flame className="size-4 text-muted-foreground" aria-hidden="true" /> Wake through my fire
                    </span>
                    <span className="mt-0.5 block text-[12px] text-muted-foreground">Adds a relay wake target so an idle cook is started when work arrives.</span>
                  </span>
                  <Switch id="ck-relay" checked={relay} onCheckedChange={setRelay} />
                </label>
              ) : (
                <p className="rounded-xl border border-border bg-muted/40 px-3.5 py-3 text-[12.5px] text-muted-foreground">
                  A cloud cook has no fire. After the token you pick how it is woken: a Claude routine, GitHub comments or a webhook.
                </p>
              )}
            </FormSheetBody>
            <FormSheetFoot>
              <Button type="submit" disabled={busy} className="max-sm:h-11">
                {busy ? <Loader2 className="animate-spin" /> : <ChefHat />} Hire the cook
              </Button>
              <Button type="button" variant="ghost" className="max-sm:h-11" disabled={busy} onClick={() => setStep("who")}>
                Back
              </Button>
            </FormSheetFoot>
          </form>
        )}

        {step === "token" && made && (
          <>
            <FormSheetBody className="pb-4">
              <TokenBox token={made.token} name={made.name} />
              <Snippets token={made.token} defaultKind={kind} />
            </FormSheetBody>
            <FormSheetFoot>
              {cloud ? (
                <>
                  <Button className="max-sm:h-11" onClick={() => setStep("wake")}>
                    I have copied it: next
                  </Button>
                  <Button variant="ghost" className="max-sm:h-11" onClick={finish}>
                    Skip the wake target
                  </Button>
                </>
              ) : (
                <Button className="max-sm:h-11" onClick={finish}>
                  I have copied it
                </Button>
              )}
            </FormSheetFoot>
          </>
        )}

        {step === "wake" && made && (
          <>
            <FormSheetBody className="pb-4">
              <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border">
                {CLOUD_WAKES.filter((w) => w.id !== "webhook" || me.is_admin).map((w) => (
                  <li key={w.id}>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void wake(w.id)}
                      className="flex min-h-14 w-full items-center gap-3 px-3.5 py-2.5 text-left hover:bg-muted/50 active:bg-muted disabled:opacity-60"
                    >
                      <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
                        <w.icon className="size-4" aria-hidden="true" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block text-[14px] font-medium">{w.title}</span>
                        <span className="block text-[12px] text-muted-foreground">{w.sub}</span>
                      </span>
                      <ChevronRight className="size-4 text-muted-foreground/70" aria-hidden="true" />
                    </button>
                  </li>
                ))}
              </ul>
              <p className="text-[12px] text-muted-foreground">Next come the steps and the form, the same page {made.name} gets from connect_cloud.</p>
            </FormSheetBody>
            <FormSheetFoot>
              <Button variant="ghost" className="max-sm:h-11" onClick={finish}>
                Do it later
              </Button>
            </FormSheetFoot>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
