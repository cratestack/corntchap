import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ExternalLink, Loader2, PanelLeftClose, PanelLeftOpen, Search, UserPlus, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { useProjects, useTeamChefs, useV1, useV1Call, type TeamRow } from "@/lib/data";
import { useNow } from "@/lib/format";
import { useIsDesktop, useIsPhone } from "@/lib/media";
import { isSuperAdmin, useSession } from "@/lib/session";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { ConvAvatar, KindChip } from "./pass-bits";
import { Composer, type Seed } from "./pass-composer";
import {
  POT,
  oneLine,
  previewKey,
  toPreview,
  useCooks,
  usePass,
  usePreviews,
  useWakeSignals,
  type CookInfo,
  type Preview,
} from "./pass-data";
import { Thread } from "./pass-thread";

/** "now", "5m", "3h", "Tue", "Nov 3": the short time of a list row. */
function shortAgo(iso: string, now: number): string {
  const d = new Date(iso);
  const s = Math.max(0, Math.round((now - d.getTime()) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  const sameDay = d.toDateString() === new Date(now).toDateString();
  if (sameDay) return `${Math.floor(s / 3600)}h`;
  if (s < 6 * 86400) return d.toLocaleDateString(undefined, { weekday: "short" });
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Fill the viewport below the page header on tablets and desktops (the phone layout is fixed between the bars). */
function useFillHeight(ref: React.RefObject<HTMLElement | null>) {
  const [h, setH] = useState<number | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const calc = () => {
      const top = el.getBoundingClientRect().top + window.scrollY;
      setH(Math.max(420, Math.round(window.innerHeight - top - 20)));
    };
    calc();
    window.addEventListener("resize", calc);
    const ro = new ResizeObserver(calc);
    ro.observe(document.body);
    return () => {
      window.removeEventListener("resize", calc);
      ro.disconnect();
    };
  }, [ref]);
  return h;
}

interface Conv {
  key: string;
  name: string;
  channel?: boolean;
  kind: "channel" | "user" | "agent";
  cook?: CookInfo | undefined;
  unread: number;
  preview?: Preview | null | undefined;
}

// ---------------------------------------------------------------- the view

export function PassView({ potKey }: { potKey: string }) {
  const me = useSession();
  const now = useNow();
  const qc = useQueryClient();
  const [params, setParams] = useSearchParams();
  const [switcher, setSwitcher] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [seed, setSeed] = useState<Seed | null>(null);
  const isPhone = useIsPhone();
  const isDesktop = useIsDesktop();
  const projects = useProjects();
  const project = projects.data?.find((p) => p.key === potKey);
  const team = project?.team ?? null;
  const members = useTeamChefs(team);
  const cooks = useCooks(now);
  const chefs = useMemo(() => (members.data ?? []).filter((m) => m.name !== me.name), [members.data, me.name]);
  const chefNames = useMemo(() => new Set(chefs.map((c) => c.name)), [chefs]);
  const taskKeys = useMemo(() => (projects.data ?? []).map((p) => p.key).sort((a, b) => b.length - a.length), [projects.data]);

  const withWho = params.get("with") || POT;
  const channel = withWho === POT;
  const select = (w: string) => {
    setSwitcher(false);
    setSeed(null);
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (w === POT) n.delete("with");
        else n.set("with", w);
        return n;
      },
      { replace: true },
    );
  };

  const refetchProjects = useRef(projects.refetch);
  refetchProjects.current = projects.refetch;
  const { page, latest, isLoading, error } = usePass(potKey, withWho, () => void refetchProjects.current());
  const signals = useWakeSignals(potKey, true);

  // the open conversation keeps its own list preview fresh
  const lastMessage = page?.messages[page.messages.length - 1];
  useEffect(() => {
    if (page) qc.setQueryData(previewKey(potKey, withWho), toPreview(lastMessage));
  }, [qc, potKey, withWho, page, lastMessage]);

  const unreadDirect = useMemo(() => {
    const m = new Map<string, number>();
    for (const d of latest?.unread.direct ?? []) if (d.chef) m.set(d.chef, d.count);
    return m;
  }, [latest]);
  const unreadPot = latest?.unread.pot ?? 0;

  // Every direct conversation you are in, the most recent first (the server lists them all).
  const directNames = useMemo(() => {
    const names = [...unreadDirect.keys()].filter((n) => n !== me.name);
    if (!channel && !names.includes(withWho)) names.unshift(withWho);
    return names;
  }, [unreadDirect, channel, withWho, me.name]);

  const previewConvs = useMemo(
    () => [
      { key: POT, unread: withWho === POT ? 0 : unreadPot, active: withWho === POT },
      ...directNames.map((n) => ({ key: n, unread: withWho === n ? 0 : (unreadDirect.get(n) ?? 0), active: withWho === n })),
    ],
    [directNames, unreadDirect, unreadPot, withWho],
  );
  const previews = usePreviews(potKey, latest ? previewConvs : []);

  const conv = (name: string, unread: number): Conv => {
    const m = members.data?.find((x) => x.name === name);
    const cook = cooks.byName.get(name);
    return {
      key: name,
      name,
      kind: (m?.kind ?? (cook ? "agent" : "user")) === "agent" ? "agent" : "user",
      cook,
      unread: withWho === name ? 0 : unread,
      preview: previews.get(name),
    };
  };
  const pinned: Conv = { key: POT, name: "pass", channel: true, kind: "channel", unread: withWho === POT ? 0 : unreadPot, preview: previews.get(POT) };
  const direct = directNames.map((n) => conv(n, unreadDirect.get(n) ?? 0));
  const others = chefs.filter((c) => !directNames.includes(c.name)).map((c) => conv(c.name, 0));

  // your own cooks that are not chefs of this pot's team
  const teams = useV1<TeamRow[]>("/teams");
  // your own cooks may be added by you, whatever your role in the team
  const canManage = isSuperAdmin(me) || !!teams.data?.find((t) => t.name === team)?.role;
  const outside = useMemo(
    () =>
      (cooks.agents ?? [])
        .filter((a) => a.owner === me.name && !chefNames.has(a.name) && a.name !== me.name)
        .map((a) => cooks.byName.get(a.name)!)
        .filter(Boolean),
    [cooks.agents, cooks.byName, me.name, chefNames],
  );

  const unreadElsewhere = (withWho === POT ? 0 : unreadPot) + [...unreadDirect.entries()].reduce((n, [k, c]) => n + (k === withWho ? 0 : c), 0);
  const withCook = channel ? undefined : cooks.byName.get(withWho);

  const compact = !isPhone && !isDesktop && collapsed;
  const rootRef = useRef<HTMLDivElement>(null);
  const fill = useFillHeight(rootRef);

  const list = (compactList: boolean) => (
    <ConvList
      active={withWho}
      select={select}
      pinned={pinned}
      direct={direct}
      others={others}
      outside={outside}
      team={team}
      canManage={canManage}
      compact={compactList}
      now={now}
    />
  );

  return (
    <div
      ref={rootRef}
      style={fill ? ({ "--pass-h": `${fill}px` } as React.CSSProperties) : undefined}
      className={cn(
        "flex min-w-0 flex-col bg-card",
        // phones: a real chat screen between the top bar and the tab bar
        "max-md:fixed max-md:inset-x-0 max-md:top-[calc(3.5rem+1px+env(safe-area-inset-top))] max-md:bottom-[calc(4rem+env(safe-area-inset-bottom))] max-md:z-20",
        // tablet and up: conversations left, thread right, filling what is left of the viewport
        "md:h-[var(--pass-h,40rem)] md:flex-row md:overflow-hidden md:rounded-[14px] md:border md:border-border",
      )}
    >
      <aside
        aria-label="Conversations"
        className={cn(
          "hidden shrink-0 flex-col border-r border-border bg-background md:flex",
          compact ? "w-[4.5rem]" : "w-64 lg:w-80",
        )}
      >
        {list(compact)}
      </aside>

      <section aria-label={channel ? "The pass" : `Conversation with ${withWho}`} className="flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex min-h-14 shrink-0 items-center gap-1 border-b border-border bg-card px-2 md:px-3">
          <button
            type="button"
            onClick={() => setCollapsed((c) => !c)}
            aria-label={collapsed ? "Show conversations" : "Hide conversations"}
            className="hidden size-9 shrink-0 place-items-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground md:max-lg:grid"
          >
            {collapsed ? <PanelLeftOpen className="size-4.5" /> : <PanelLeftClose className="size-4.5" />}
          </button>
          <button
            type="button"
            onClick={() => setSwitcher(true)}
            aria-label="Switch conversation"
            className="relative flex min-h-12 min-w-0 flex-1 items-center gap-3 rounded-xl px-2 text-left active:bg-muted md:pointer-events-none md:min-h-0 md:px-1"
          >
            <ConvAvatar name={withWho} channel={channel} cook={withCook} size="size-9" ring="ring-card" />
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5 font-display text-[17px] leading-tight">
                <span className="truncate">{channel ? "pass" : withWho}</span>
                <ChevronDown className="size-4 shrink-0 text-muted-foreground md:hidden" aria-hidden="true" />
              </span>
              <HeaderSubtitle channel={channel} potKey={potKey} chefCount={(members.data ?? []).length} who={withWho} cook={withCook} person={!channel && !withCook} />
            </span>
            {unreadElsewhere > 0 && (
              <span className="absolute top-2 left-9 size-2.5 rounded-full bg-primary ring-2 ring-card md:hidden">
                <span className="sr-only">{unreadElsewhere} unread elsewhere</span>
              </span>
            )}
          </button>
          {withCook && (
            <Link
              to={`/cooks/${withWho}`}
              className="hidden h-8 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-[12.5px] text-muted-foreground hover:bg-muted hover:text-foreground md:inline-flex"
            >
              Cook page <ExternalLink className="size-3.5" aria-hidden="true" />
            </Link>
          )}
        </header>

        <Thread
          key={`${potKey}/${withWho}`}
          potKey={potKey}
          messages={page?.messages}
          loading={isLoading}
          error={error}
          me={me.name}
          who={channel ? null : withWho}
          cooks={cooks.byName}
          chefNames={chefNames}
          taskKeys={taskKeys}
          signals={signals}
          actions={{ reply: (text) => setSeed({ text, n: (seed?.n ?? 0) + 1 }), direct: select }}
        />
        <Composer potKey={potKey} withWho={withWho} chefs={chefs} cook={withCook} seed={seed} />
      </section>

      <Sheet open={switcher} onOpenChange={setSwitcher}>
        <SheetContent side="bottom" className="h-[88dvh] max-h-[88dvh] gap-0 px-0 pb-0 md:hidden">
          <SheetTitle className="px-4 pb-2 font-display text-[18px]">Conversations</SheetTitle>
          <SheetDescription className="sr-only">Pick the pass, or a chef to write to directly</SheetDescription>
          <div className="flex min-h-0 flex-1 flex-col pb-[env(safe-area-inset-bottom)]">{list(false)}</div>
        </SheetContent>
      </Sheet>
    </div>
  );
}

function HeaderSubtitle({
  channel,
  potKey,
  chefCount,
  who,
  cook,
  person,
}: {
  channel: boolean;
  potKey: string;
  chefCount: number;
  who: string;
  cook: CookInfo | undefined;
  person: boolean;
}) {
  const cls = "flex min-w-0 items-center gap-1.5 truncate text-[12px] text-muted-foreground";
  if (channel)
    return (
      <span className={cls}>
        Everyone in {potKey}
        {chefCount > 0 && ` · ${chefCount} ${chefCount === 1 ? "chef" : "chefs"}`}
      </span>
    );
  if (cook) {
    const live = cook.state === "cooking" || cook.state === "awake";
    return (
      <span className={cls}>
        <span className="truncate">
          {cook.agentKind} · {cook.cloud ? "cloud" : "local"} ·{" "}
          <span className={cn(live && "font-medium text-corn-text")}>{cook.cloud ? (live ? "awake, " : "") + "fire and forget" : cook.state}</span>
        </span>
      </span>
    );
  }
  return <span className={cls}>{person ? `Person · only you and ${who} read this` : ""}</span>;
}

// ---------------------------------------------------------------- the list

function ConvList({
  active,
  select,
  pinned,
  direct,
  others,
  outside,
  team,
  canManage,
  compact,
  now,
}: {
  active: string;
  select: (w: string) => void;
  pinned: Conv;
  direct: Conv[];
  others: Conv[];
  outside: CookInfo[];
  team: string | null;
  canManage: boolean;
  compact: boolean;
  now: number;
}) {
  const [q, setQ] = useState("");
  const [allOut, setAllOut] = useState(false);
  const total = 1 + direct.length + others.length + outside.length;
  const needle = q.trim().toLowerCase();
  const hit = (n: string) => !needle || n.toLowerCase().includes(needle);
  const d = direct.filter((c) => hit(c.name));
  const o = others.filter((c) => hit(c.name));
  const outAll = outside.filter((c) => hit(c.name));
  const out = allOut || needle ? outAll : outAll.slice(0, 3);
  const showPinned = hit("pass");
  const searchable = total > 8 && !compact;

  const row = (c: Conv, fresh = false) => (
    <ConvRow key={c.key} c={c} active={active === c.key} compact={compact} now={now} fresh={fresh} onClick={() => select(c.key)} />
  );

  return (
    <>
      {searchable && (
        <div className="shrink-0 px-3 pt-3 pb-1">
          <label className="relative block">
            <span className="sr-only">Search conversations</span>
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <input
              type="search"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search chefs"
              className="h-10 w-full rounded-xl border border-transparent bg-muted pr-8 pl-8 text-base outline-none placeholder:text-muted-foreground focus:border-border focus:bg-card md:h-9 md:text-[13.5px]"
            />
            {q && (
              <button type="button" onClick={() => setQ("")} aria-label="Clear search" className="absolute top-1/2 right-1.5 grid size-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-background">
                <X className="size-3.5" />
              </button>
            )}
          </label>
        </div>
      )}
      <ul className="scrollbar-thin min-h-0 flex-1 space-y-0.5 overflow-y-auto overscroll-contain p-2">
        {showPinned && row(pinned)}
        {d.length > 0 && !compact && <SectionHead>Direct</SectionHead>}
        {compact && d.length > 0 && <li className="mx-3 my-1.5 h-px list-none bg-border" aria-hidden="true" />}
        {d.map((c) => row(c))}
        {o.length > 0 && !compact && <SectionHead hint="No messages yet">Start a conversation</SectionHead>}
        {compact && o.length > 0 && <li className="mx-3 my-1.5 h-px list-none bg-border" aria-hidden="true" />}
        {o.map((c) => row(c, true))}
        {out.length > 0 && !compact && (
          <SectionHead hint="Yours, but not chefs of this pot">Your cooks outside this team</SectionHead>
        )}
        {!compact && out.map((c) => <OutsideRow key={c.name} cook={c} team={team} canManage={canManage} />)}
        {!compact && outAll.length > out.length && (
          <li className="list-none px-3 py-1">
            <button type="button" onClick={() => setAllOut(true)} className="text-[12.5px] font-medium text-primary underline-offset-2 hover:underline">
              Show {outAll.length - out.length} more of your cooks
            </button>
          </li>
        )}
        {needle && !showPinned && d.length + o.length + outAll.length === 0 && (
          <li className="list-none px-3 py-6 text-center text-[13px] text-muted-foreground">No chef matches “{q}”.</li>
        )}
      </ul>
    </>
  );
}

function SectionHead({ children, hint }: { children: React.ReactNode; hint?: string }) {
  return (
    <li className="list-none px-3 pt-3.5 pb-1" title={hint}>
      <span className="label-caps">{children}</span>
    </li>
  );
}

function ConvRow({ c, active, compact, now, fresh, onClick }: { c: Conv; active: boolean; compact: boolean; now: number; fresh: boolean; onClick: () => void }) {
  const cook = c.cook;
  const newCount = c.unread;
  let sub: React.ReactNode;
  if (newCount > 0) sub = <span className="font-medium text-foreground">{newCount === 1 ? "1 new message" : `${newCount} new messages`}</span>;
  else if (c.preview) sub = <>{c.preview.from && c.preview.from !== c.name && !c.channel ? "you: " : c.channel && c.preview.from ? `${c.preview.from}: ` : ""}{oneLine(c.preview.body)}</>;
  else if (c.channel) sub = "Everyone in the pot";
  else if (cook) sub = `${cook.agentKind} · ${cook.cloud ? "cloud" : "local"} · ${cook.cloud ? "fire and forget" : cook.state}`;
  else sub = fresh ? "Person" : "No messages yet";

  return (
    <li className="list-none">
      <button
        type="button"
        onClick={onClick}
        aria-current={active ? "true" : undefined}
        title={compact ? c.name : undefined}
        className={cn(
          "relative flex w-full items-center rounded-xl text-left transition-colors hover:bg-muted/70 active:bg-muted",
          compact ? "min-h-14 justify-center px-2" : "min-h-[3.75rem] gap-3 px-3 py-2",
          active && "bg-muted",
        )}
      >
        <ConvAvatar name={c.name} channel={!!c.channel} cook={cook} size="size-10" ring={active ? "ring-muted" : "ring-background"} className="" />
        {compact ? (
          newCount > 0 && (
            <span className="absolute top-1.5 right-2 inline-flex min-w-4.5 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10px] leading-[18px] font-bold text-primary-foreground">
              {newCount}
            </span>
          )
        ) : (
          <span className="min-w-0 flex-1">
            <span className="flex items-baseline gap-2">
              <span className={cn("truncate text-[14.5px]", newCount > 0 ? "font-semibold" : "font-medium")}>{c.name}</span>
              {c.kind === "agent" && !c.channel && cook === undefined && <KindChip>cook</KindChip>}
              {c.preview && (
                <span className={cn("ml-auto shrink-0 text-[11px]", newCount > 0 ? "font-medium text-primary" : "text-muted-foreground")}>{shortAgo(c.preview.at, now)}</span>
              )}
            </span>
            <span className="mt-0.5 flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-[12.5px] text-muted-foreground">{sub}</span>
              {newCount > 0 && (
                <span className="inline-flex min-w-5 shrink-0 items-center justify-center rounded-full bg-primary px-1.5 font-mono text-[11px] leading-5 font-bold text-primary-foreground">
                  <span aria-hidden="true">{newCount}</span>
                  <span className="sr-only"> unread</span>
                </span>
              )}
            </span>
          </span>
        )}
      </button>
    </li>
  );
}

/** One of your cooks that is not a chef of this pot's team: greyed, with why, and a way to fix it for a team owner. */
function OutsideRow({ cook, team, canManage }: { cook: CookInfo; team: string | null; canManage: boolean }) {
  const call = useV1Call();
  const [busy, setBusy] = useState(false);
  async function add() {
    if (!team) return;
    setBusy(true);
    try {
      await call("POST", `/teams/${team}/members`, { principal: cook.name, role: "member" });
      toast(`${cook.name} joined ${team}`);
    } catch (e) {
      toast.error(`Could not add ${cook.name}`, { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <li className="list-none rounded-xl px-3 py-2">
      <div className="flex items-center gap-3 opacity-90">
        <span className="opacity-55 grayscale">
          <ConvAvatar name={cook.name} cook={cook} size="size-10" ring="ring-background" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[14.5px] font-medium text-muted-foreground">{cook.name}</span>
          <span className="block text-[12px] leading-snug text-muted-foreground">
            Not in {team ?? "this pot's"} team yet.
          </span>
        </span>
      </div>
      {canManage && team && (
        <Button type="button" variant="outline" size="sm" onClick={() => void add()} disabled={busy} className="mt-2 ml-[3.25rem] pointer-coarse:min-h-10">
          {busy ? <Loader2 className="animate-spin" /> : <UserPlus />} Add to the team
        </Button>
      )}
    </li>
  );
}
