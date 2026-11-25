import { useEffect, useMemo, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { ChefHat, ChevronRight, Cloud, Flame, Laptop, Loader2, Server, Minus, Plus, MessageSquare, Webhook } from "lucide-react";
import { toast } from "sonner";
import type { PrincipalView, TeamView } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { Avatar, FormSheetBody, FormSheetFoot, FormSheetHead, LabelChip, StatePill, StateDot, STATE_LABEL, formSheetClass } from "@/components/bits";
import { fmtTokens } from "@/components/usage";
import { Snippets, TokenBox } from "@/components/secret-dialog";
import { FilterBar } from "@/components/filter-bar";
import { ListPage } from "@/components/list-page";
import { PageTools } from "@/components/page-tools";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { type VirtualTableColumn } from "@/components/ui/virtual-table";
import { useClaimedTasks, useProc, useProcCall, useProjects, useV1Call } from "@/lib/data";
import { useCooksSource, useTeamsSource } from "@/lib/sources";
import { DISABLED_OPTIONS, disabledArg, principalState, staticFacet, useCookByName, usePersonFacet, useTeamFacet } from "@/lib/people";
import { useUrlFilters, type UrlFilterSchema } from "@/lib/url-filters";
import { useStoreValue } from "@/lib/use-store-value";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { KIND_LABEL } from "@/lib/snippets";
import { canOrchestrate, isSuperAdmin, useSession } from "@/lib/session";

const COOK_FILTERS = { team: { type: "string" }, owner: { type: "string" }, disabled: { type: "string" } } as const satisfies UrlFilterSchema;
const cookPk = (a: PrincipalView) => a.name;
const teamPk = (t: TeamView) => t.name;

export function CooksPage() {
  const me = useSession();
  const f = useUrlFilters(COOK_FILTERS);
  const teamFacet = useTeamFacet();
  const ownerFacet = usePersonFacet("owner", "Owner", canOrchestrate(me));
  const facets = [teamFacet, ...(canOrchestrate(me) ? [ownerFacet] : []), staticFacet("disabled", "State", DISABLED_OPTIONS)];
  const cooks = useCooksSource({
    q: f.q,
    ...(f.values.team ? { team: f.values.team } : {}),
    ...(f.values.owner ? { owner: f.values.owner } : {}),
    ...disabledArg(f.values.disabled),
  });
  const projects = useProjects();
  const claimed = useClaimedTasks(projects.data);
  const usage = useProc("usageReport", { days: 7 });
  const [creating, setCreating] = useState(false);
  const loc = useLocation();
  const navigate = useNavigate();
  const now = useNow();

  // old links to /cooks#name now land on the cook's page
  const hashed = useCookByName(loc.hash.slice(1), !!loc.hash);
  useEffect(() => {
    if (hashed.item) void navigate(`/cooks/${hashed.item.name}`, { replace: true });
  }, [hashed.item, navigate]);

  const claimedTasks = claimed.tasks;
  const current = useMemo(() => {
    const m = new Map<string, (typeof claimedTasks)[number]>();
    for (const t of claimedTasks) if (t.claimed_by && t.lease_live) m.set(t.claimed_by, t);
    return m;
  }, [claimedTasks]);

  const columns: VirtualTableColumn<PrincipalView>[] = [
    {
      id: "cook",
      header: "Cook",
      cell: (a) => (
        <Link to={`/cooks/${a.name}`} aria-label={`Cook ${a.name}, ${STATE_LABEL[principalState(a, now)]}`} className="flex items-center gap-3 outline-none hover:underline focus-visible:underline">
          <Avatar name={a.name} className="size-9 rounded-lg text-[16px]" />
          <span className="min-w-0">
            <span className="flex items-center gap-2">
              <span className="truncate font-display text-[16px] leading-tight">{a.name}</span>
              {a.runtime === "cloud" && (
                <LabelChip className="gap-1 px-1.5">
                  <Cloud className="size-3" aria-hidden="true" /> {a.fireAndForget ? "fire and forget" : "cloud"}
                </LabelChip>
              )}
            </span>
            <span className="block truncate text-[12px] text-muted-foreground">
              {KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind ?? "Agent"} · {a.runtime ?? "local"}
            </span>
          </span>
        </Link>
      ),
    },
    { id: "state", header: "State", cell: (a) => <StatePill state={principalState(a, now)} /> },
    {
      id: "cooking",
      header: "Cooking",
      cell: (a) => {
        const id = current.get(a.name)?.id;
        return id ? <span className="font-mono text-[12px] font-bold">{id}</span> : <span className="text-muted-foreground">nothing right now</span>;
      },
    },
    {
      id: "leases",
      header: "Leases",
      cell: (a) => (
        <span className="flex items-center gap-2 font-mono text-[12px]">
          <span>
            <b>{a.busy}</b>/{a.max_leases}
          </span>
          <LeaseBlocks busy={a.busy} max={a.max_leases} />
        </span>
      ),
    },
    { id: "seen", header: "Last seen", className: "text-[12.5px]", cell: (a) => fmtAgo(a.last_seen_at ?? null, now) },
    {
      id: "teams",
      header: "Teams",
      cell: (a) =>
        a.teams.length === 0 ? (
          <span className="text-muted-foreground">none</span>
        ) : (
          <span className="flex min-w-0 flex-wrap gap-1">
            {a.teams.map((t) => (
              <LabelChip key={t}>{t}</LabelChip>
            ))}
          </span>
        ),
    },
  ];

  return (
    <>
      <ListPage<PrincipalView>
        title="Cooks"
        actions={
          <>
            <PageTools />
            <Button onClick={() => setCreating(true)}>
              <Plus /> New cook
            </Button>
          </>
        }
        filters={
          <div className="space-y-3">
            <p className="max-w-2xl text-[13.5px] text-muted-foreground">
              The agents that scoop and serve your tasks.
              {usage.data && (
                <span className="mt-1 block" title={usage.data.note}>
                  Tokens sent, 7 days: <span className="font-mono font-bold">~{fmtTokens(usage.data.totals.tokens)}</span> (an estimate)
                </span>
              )}
            </p>
            <FilterBar
              q={f.q}
              onQ={f.setQ}
              facets={facets}
              values={{ team: f.values.team, owner: f.values.owner, disabled: f.values.disabled }}
              onChange={(p) => f.set(p as never)}
              onClear={f.clear}
              searchPlaceholder="Find a cook"
            />
          </div>
        }
        source={cooks}
        itemKey={cookPk}
        columns={columns}
        renderItem={(a) => <CookRow a={a} current={current.get(a.name)?.id ?? null} now={now} />}
        empty={
          f.q || f.active > 0 ? (
            <>
              No cook matches these filters.{" "}
              <Button variant="link" className="h-auto p-0" onClick={f.clear}>
                Show all cooks
              </Button>
            </>
          ) : (
            <>
              No cooks yet. A cook is an agent (OpenCode, Claude Code, Codex, Copilot, Antigravity or any MCP client). Make one, copy its token or MCP config, and it can start scooping.{" "}
              <Button variant="link" className="h-auto p-0" onClick={() => setCreating(true)}>
                Hire the first cook
              </Button>
            </>
          )
        }
      />
      <NewCookSheet open={creating} onOpenChange={setCreating} />
    </>
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

/** One cook as a two-line row on phones (tablets and up get the table). The whole row opens the cook's page. */
function CookRow({ a, current, now }: { a: PrincipalView; current: string | null; now: number }) {
  const st = principalState(a, now);
  const cloud = a.runtime === "cloud";
  const kind = KIND_LABEL[a.agent_kind ?? "other"] ?? a.agent_kind ?? "Agent";
  return (
    <Link
      to={`/cooks/${a.name}`}
      aria-label={`Cook ${a.name}, ${STATE_LABEL[st]}`}
      className={cn("flex min-h-16 items-center gap-3 px-4 py-3 outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/60 active:bg-muted", a.disabled && "opacity-75")}
    >
      <Avatar name={a.name} className="size-10 rounded-lg text-[18px]" />
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-2">
          <span className="truncate font-display text-[17px] leading-tight">{a.name}</span>
          {cloud && (
            <LabelChip className="gap-1 px-1.5">
              <Cloud className="size-3" aria-hidden="true" /> {a.fireAndForget ? "fire and forget" : "cloud"}
            </LabelChip>
          )}
        </p>
        <p className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
          {kind} · {a.runtime ?? "local"}
        </p>
        <p className="mt-0.5 flex items-center gap-1.5 text-[12.5px]">
          <StateDot state={st} className={st === "idle" ? "bg-muted-foreground/50" : ""} />
          <span className={cn(st === "disabled" ? "text-destructive" : st === "idle" ? "text-muted-foreground" : "text-foreground")}>{STATE_LABEL[st]}</span>
          {current && <span className="font-mono text-[12px] font-bold">{current}</span>}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <span className="text-right font-mono text-[12.5px] leading-tight">
          <span className="font-bold">{a.busy}</span>
          <span className="text-muted-foreground">/{a.max_leases}</span>
          <span className="block text-[10px] uppercase tracking-wider text-muted-foreground">leases</span>
        </span>
        <ChevronRight className="size-4 text-muted-foreground/70" aria-hidden="true" />
      </div>
    </Link>
  );
}

// ------------------------------------------------------------------ New cook: a step-by-step sheet

type Step = "who" | "where" | "token" | "wake";

const CLOUD_WAKES = [
  { id: "claude-routine", title: "Claude Code on the web", sub: "A routine is started on each wake", icon: Cloud },
  { id: "github-comment", title: "GitHub comment", sub: "Codex cloud, Copilot and others", icon: MessageSquare },
  { id: "webhook", title: "Webhook", sub: "Your own endpoint (super-admins)", icon: Webhook },
  { id: "coder", title: "Coder", sub: "An agent run on your Coder deployment (super-admins)", icon: Server },
  { id: "devin", title: "Devin", sub: "A Devin session on your Devin organization", icon: Server },
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
  const teamQ = useStoreValue("");
  const teams = useTeamsSource(teamQ.value, { enabled: open });
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
    if (open) setTeam((t) => t || (teams.items.find((x) => x.role === "owner")?.name ?? teams.items[0]?.name) || "");
  }, [open, teams.items, me.role]);

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
            {step === "wake" && `${made?.name} runs in the cloud and is fire and forget: started for ready work (and its owner's direct messages), it opens a pull request and stops. Pick how.`}
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
                      <span className="block text-[12px] text-muted-foreground">Started for ready work or a mention</span>
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
                <VirtualCombobox<TeamView>
                  id="ck-team"
                  label="Team"
                  placeholder="No team yet: it sees no pot"
                  source={teams}
                  onQuery={teamQ.set}
                  itemKey={teamPk}
                  itemText={teamPk}
                  nullOption="No team: it sees no pot"
                  value={team || null}
                  onChange={(v) => setTeam(v ?? "")}
                  className="h-11"
                />
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
                  A cloud cook has no fire. After the token you pick how it is woken: a Claude routine, GitHub comments, a webhook, Coder or Devin.
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
                {CLOUD_WAKES.filter((w) => (w.id !== "webhook" && w.id !== "coder") || isSuperAdmin(me)).map((w) => (
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
