import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { Link, useSearchParams } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import type { MemberView, PassConversationView } from "@corntchap/api";
import { ChevronDown, ExternalLink, Loader2, MessageSquarePlus, PanelLeftClose, PanelLeftOpen, Search, UserPlus, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { useProjects, useV1Call } from "@/lib/data";
import { useNow } from "@/lib/format";
import type { ListSource } from "@/lib/listbox";
import { useListQuery } from "@/lib/list-query";
import { useIsDesktop, useIsPhone } from "@/lib/media";
import { isSuperAdmin, useSession } from "@/lib/session";
import { useCooksSource, useTeamMembersSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { useUrlFilters } from "@/lib/url-filters";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { VirtualList } from "@/components/ui/virtual-list";
import { VirtualMenu } from "@/components/ui/virtual-menu";
import { Avatar } from "./bits";
import { ConvAvatar, KindChip } from "./pass-bits";
import { Composer, type Seed } from "./pass-composer";
import {
  POT,
  conversationPreview,
  oneLine,
  previewKey,
  toPreview,
  useConversations,
  useCooks,
  useMarkConversationRead,
  usePass,
  usePotPreview,
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

/** Sets `--pass-h` on the element: the height that fills the viewport below the page header (the thread pane's height). */
function useFillHeight(ref: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const calc = () => {
      const top = el.getBoundingClientRect().top + window.scrollY;
      el.style.setProperty("--pass-h", `${Math.max(420, Math.round(window.innerHeight - top - 20))}px`);
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
}

/**
 * The pass screen. Phones: a chat screen fixed between the top bar and the tab bar (the thread is the one thing that
 * scrolls). Tablets and up: the conversation list and the thread side by side; the page scrolls, so a long list runs
 * down the page while the thread (a chat pane, scrolling inside itself) stays in view beside it.
 */
export function PassFrame({ aside, compact, children, extra }: { aside: ReactNode; compact: boolean; children: ReactNode; extra?: ReactNode }) {
  const rootRef = useRef<HTMLDivElement>(null);
  useFillHeight(rootRef);
  return (
    <div
      ref={rootRef}
      className={cn(
        "flex min-w-0 flex-col bg-card",
        // phones: a real chat screen between the top bar and the tab bar
        "max-md:fixed max-md:inset-x-0 max-md:top-[calc(3.5rem+1px+env(safe-area-inset-top))] max-md:bottom-[calc(4rem+env(safe-area-inset-bottom))] max-md:z-20",
        // tablet and up: conversations left (as tall as they are), thread right (as tall as what is left of the viewport)
        "md:flex-row md:items-start md:overflow-clip md:rounded-[14px] md:border md:border-border",
      )}
    >
      <aside aria-label="Conversations" className={cn("hidden shrink-0 flex-col self-stretch border-r border-border bg-background md:flex", compact ? "w-[4.5rem]" : "w-64 lg:w-80")}>
        {aside}
      </aside>
      {children}
      {extra}
    </div>
  );
}

export interface Conv {
  key: string;
  name: string;
  channel?: boolean;
  kind: "channel" | "user" | "agent";
  cook?: CookInfo | undefined;
  unread: number;
  preview?: Preview | null | undefined;
}

// ---------------------------------------------------------------- the view

/** The conversation list's filters in the URL: the search text and "unread only". */
const CHAT_FILTERS = { chat: { type: "string" as const }, unread: { type: "bool" as const } };

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
  const filters = useUrlFilters(CHAT_FILTERS);

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

  // the team's chefs as far as they are loaded: who a message author may be written to, and how many there are
  const members = useTeamMembersSource(team ?? "", { enabled: !!team });
  const cooks = useCooks(now, team, channel ? null : withWho);
  const conversations = useConversations(potKey, { q: filters.values.chat, unread: filters.values.unread });
  const opened = conversations.items.find((c) => c.chef === withWho);
  // a conversation not started yet (picked from "Message a chef"): who it is, by name
  const named = useTeamMembersSource(team ?? "", { q: withWho, enabled: !channel && !!team && !opened });
  const chefNames = useMemo(() => {
    const names = new Set<string>([...members.items.map((m) => m.name), ...conversations.items.map((c) => c.chef), ...cooks.byName.keys()]);
    names.delete(me.name);
    return names;
  }, [members.items, conversations.items, cooks.byName, me.name]);
  const withKind = opened?.kind ?? named.items.find((m) => m.name === withWho)?.kind ?? (cooks.byName.has(withWho) ? "agent" : "user");
  const taskKeys = useMemo(() => (projects.data ?? []).map((p) => p.key).sort((a, b) => b.length - a.length), [projects.data]);

  const markRead = useMarkConversationRead();
  const refetchProjects = useRef(projects.refetch);
  refetchProjects.current = projects.refetch;
  const { page, latest, isLoading, error, hasOlder, loadingOlder, loadOlder } = usePass(potKey, withWho, () => {
    void refetchProjects.current();
    markRead(withWho);
  });
  const signals = useWakeSignals(potKey, true);

  // the open conversation keeps its own list preview fresh (the pass's; a direct one comes with its row)
  const lastMessage = page?.messages[page.messages.length - 1];
  useEffect(() => {
    if (page && withWho === POT) qc.setQueryData(previewKey(potKey, POT), toPreview(lastMessage));
  }, [qc, potKey, withWho, page, lastMessage]);

  const unreadDirect = useMemo(() => {
    const m = new Map<string, number>();
    for (const d of latest?.unread.direct ?? []) if (d.chef) m.set(d.chef, d.count);
    return m;
  }, [latest]);
  const unreadPot = latest?.unread.pot ?? 0;
  const potPreview = usePotPreview(potKey, !!latest && unreadPot === 0 && withWho !== POT);

  const conv = (c: PassConversationView): Conv => ({
    key: c.chef,
    name: c.chef,
    kind: c.kind === "agent" ? "agent" : "user",
    cook: cooks.byName.get(c.chef),
    unread: withWho === c.chef ? 0 : c.unread,
    preview: conversationPreview(c),
  });
  const pinned: Conv = { key: POT, name: "pass", channel: true, kind: "channel", unread: withWho === POT ? 0 : unreadPot, preview: potPreview };
  const rows = useMemo<ListSource<Conv>>(
    () => ({ ...conversations, items: conversations.items.map(conv) }),
    // `conv` follows these
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [conversations, cooks.byName, withWho],
  );
  // the open conversation, until it has a row of its own
  const current: Conv | null =
    channel || opened || filters.values.chat || filters.values.unread
      ? null
      : { key: withWho, name: withWho, kind: withKind === "agent" ? "agent" : "user", cook: cooks.byName.get(withWho), unread: 0, preview: null };

  // your own cooks may be added by you, whatever your role in the team
  const myTeam = useListQuery("teamPage", {}, { q: team ?? "", enabled: !!team });
  const canManage = isSuperAdmin(me) || !!myTeam.items.find((t) => t.name === team)?.role;

  const unreadElsewhere = (withWho === POT ? 0 : unreadPot) + [...unreadDirect.entries()].reduce((n, [k, c]) => n + (k === withWho ? 0 : c), 0);
  const withCook = channel ? undefined : cooks.byName.get(withWho);

  const compact = !isPhone && !isDesktop && collapsed;
  const list = (compactList: boolean, scrolls = false) => (
    <ConvList
      active={withWho}
      select={select}
      pinned={pinned}
      current={current}
      source={rows}
      q={filters.values.chat}
      onQ={(chat) => filters.set({ chat })}
      unread={filters.values.unread}
      onUnread={(unread) => filters.set({ unread })}
      tools={
        <>
          <StartConversation team={team} select={select} />
          <OutsideCooks team={team} canManage={canManage} />
        </>
      }
      compact={compactList}
      now={now}
      scrolls={scrolls}
    />
  );

  return (
    <PassFrame
      compact={compact}
      aside={isPhone ? null : list(compact)}
      extra={
        <Sheet open={switcher} onOpenChange={setSwitcher}>
          <SheetContent side="bottom" className="h-[88dvh] max-h-[88dvh] gap-0 px-0 pb-0 md:hidden">
            <SheetTitle className="px-4 pb-2 font-display text-[18px]">Conversations</SheetTitle>
            <SheetDescription className="sr-only">Pick the pass, or a chef to write to directly</SheetDescription>
            <div className="flex min-h-0 flex-1 flex-col pb-[env(safe-area-inset-bottom)]">{list(false, true)}</div>
          </SheetContent>
        </Sheet>
      }
    >
      <section aria-label={channel ? "The pass" : `Conversation with ${withWho}`} className="flex min-h-0 min-w-0 flex-1 flex-col md:sticky md:top-5 md:h-[var(--pass-h,40rem)]">
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
              <HeaderSubtitle channel={channel} potKey={potKey} chefCount={members.total ?? members.items.length} who={withWho} cook={withCook} person={!channel && !withCook} />
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
          hasOlder={hasOlder}
          loadingOlder={loadingOlder}
          onLoadOlder={loadOlder}
        />
        <Composer potKey={potKey} withWho={withWho} team={team} agent={withKind === "agent"} cook={withCook} seed={seed} />
      </section>
    </PassFrame>
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

/**
 * The conversations: the pass pinned, then your direct conversations, most recent first, a page at a time and
 * searchable on the server. As tall as the rows loaded and carried by the page (beside the thread), or, with
 * `scrolls`, filling a sheet and scrolling inside it.
 */
export function ConvList({
  active,
  select,
  pinned,
  current = null,
  source,
  q,
  onQ,
  unread,
  onUnread,
  tools,
  compact,
  now,
  scrolls = false,
}: {
  active: string;
  select: (w: string) => void;
  pinned: Conv;
  /** The open conversation when it has no row yet (nothing said so far). */
  current?: Conv | null;
  source: ListSource<Conv>;
  /** The search text, as typed, and a change of it. */
  q: string;
  onQ: (q: string) => void;
  unread: boolean;
  onUnread: (unread: boolean) => void;
  /** Buttons under the search: start a conversation, add one of your cooks. */
  tools?: ReactNode;
  compact: boolean;
  now: number;
  scrolls?: boolean;
}) {
  const searching = !!q.trim() || unread;
  const showPinned = !searching || "pass".includes(q.trim().toLowerCase());
  const loadMore = () => {
    if (source.hasNextPage && !source.isFetchingNextPage) source.fetchNextPage();
  };
  const footer = source.isFetchingNextPage ? (
    <p role="status" className="flex items-center justify-center gap-2 py-3 text-[12px] text-muted-foreground">
      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
    </p>
  ) : source.capped ? (
    <p className="px-3 py-3 text-center text-[12px] text-muted-foreground">That is as far as this goes. Search to find a conversation.</p>
  ) : null;
  const empty =
    source.isLoading && source.items.length === 0 ? (
      <p role="status" aria-busy="true" className="px-3 py-6 text-center text-[13px] text-muted-foreground">
        Loading
      </p>
    ) : source.error && source.items.length === 0 ? (
      <p role="alert" className="px-3 py-6 text-center text-[13px] text-destructive">
        Could not load the conversations.
      </p>
    ) : (
      <p className="px-3 py-6 text-center text-[13px] text-muted-foreground">
        {searching ? (q.trim() ? `No conversation matches “${q.trim()}”.` : "Nothing unread.") : compact ? "" : "No conversations yet. Message a chef to start one."}
      </p>
    );

  return (
    <>
      {!compact && (
        <div className="shrink-0 space-y-1.5 px-3 pt-3 pb-1">
          <div className="flex items-center gap-1.5">
            <label className="relative block min-w-0 flex-1">
              <span className="sr-only">Search conversations</span>
              <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
              <input
                type="search"
                value={q}
                onChange={(e) => onQ(e.target.value)}
                placeholder="Search chefs"
                autoComplete="off"
                className="h-10 w-full rounded-xl border border-transparent bg-muted pr-8 pl-8 text-base outline-none placeholder:text-muted-foreground focus:border-border focus:bg-card md:h-9 md:text-[13.5px]"
              />
              {q && (
                <button type="button" onClick={() => onQ("")} aria-label="Clear search" className="absolute top-1/2 right-1.5 grid size-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-background">
                  <X className="size-3.5" />
                </button>
              )}
            </label>
            <button
              type="button"
              aria-pressed={unread}
              onClick={() => onUnread(!unread)}
              className={cn(
                "h-10 shrink-0 rounded-xl border px-2.5 text-[12.5px] transition-colors md:h-9",
                unread ? "border-primary/60 bg-primary/10 font-medium text-primary" : "border-transparent bg-muted text-muted-foreground hover:text-foreground",
              )}
            >
              Unread
            </button>
          </div>
          {tools}
        </div>
      )}
      <div className="shrink-0 space-y-0.5 px-2 pt-2">
        {showPinned && <ConvRow c={pinned} active={active === pinned.key} compact={compact} now={now} fresh={false} onClick={() => select(pinned.key)} />}
        {current && <ConvRow c={current} active={active === current.key} compact={compact} now={now} fresh onClick={() => select(current.key)} />}
      </div>
      {!compact && source.items.length > 0 && <SectionHead>Direct</SectionHead>}
      <div className={cn("pb-2", scrolls && "min-h-0 flex-1")}>
        <VirtualList<Conv>
          items={source.items}
          itemKey={(c) => c.key}
          renderItem={(c) => <ConvRow c={c} active={active === c.key} compact={compact} now={now} fresh={false} onClick={() => select(c.key)} />}
          scroll={scrolls ? "own" : "window"}
          onEndReached={loadMore}
          footer={footer}
          empty={empty}
          itemClassName="px-2 py-px"
          defaultItemHeight={60}
          aria-label="Direct conversations"
        />
      </div>
    </>
  );
}

function SectionHead({ children, hint }: { children: React.ReactNode; hint?: string }) {
  return (
    <div className="shrink-0 px-5 pt-3.5 pb-1" title={hint}>
      <span className="label-caps">{children}</span>
    </div>
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
  else sub = fresh ? "No messages yet" : "";

  return (
    <div>
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
    </div>
  );
}

const TOOL =
  "flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-left text-[12.5px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none pointer-coarse:min-h-11";

/** Start a conversation: a menu of the team's chefs (searched on the server), picking one opens it. */
function StartConversation({ team, select }: { team: string | null; select: (name: string) => void }) {
  const me = useSession();
  const query = useStoreValue("");
  const members = useTeamMembersSource(team ?? "", { q: query.value, enabled: !!team });
  const source = useMemo<ListSource<MemberView>>(() => ({ ...members, items: members.items.filter((m) => m.name !== me.name) }), [members, me.name]);
  if (!team) return null;
  return (
    <VirtualMenu<MemberView>
      label="Message a chef"
      mode="action"
      source={source}
      onQuery={query.set}
      itemKey={(m) => m.name}
      onSelect={(m) => select(m.name)}
      renderItem={(m) => (
        <span className="flex min-h-9 items-center gap-2.5 text-[14px]">
          <Avatar name={m.name} className="rounded-lg" />
          <span className="min-w-0 flex-1 truncate">
            <span className="font-medium">{m.name}</span>
            {m.display_name && <span className="ml-1.5 text-[12px] text-muted-foreground">{m.display_name}</span>}
          </span>
          {m.kind === "agent" && <KindChip>cook</KindChip>}
        </span>
      )}
      trigger={
        <button type="button" className={TOOL}>
          <MessageSquarePlus className="size-4 shrink-0" aria-hidden="true" /> Message a chef
        </button>
      }
    />
  );
}

/** Your cooks that are not chefs of this pot's team: greyed, with why, and a way to fix it for a team owner. */
function OutsideCooks({ team, canManage }: { team: string | null; canManage: boolean }) {
  const me = useSession();
  const call = useV1Call();
  const query = useStoreValue("");
  const cooks = useCooksSource({ owner: me.name, q: query.value, enabled: !!team });
  const source = useMemo(() => ({ ...cooks, items: cooks.items.filter((c) => !!team && !c.teams.includes(team) && c.name !== me.name) }), [cooks, team, me.name]);
  if (!team) return null;
  async function add(cook: string) {
    try {
      await call("POST", `/teams/${team}/members`, { principal: cook, role: "member" });
      toast(`${cook} joined ${team}`);
    } catch (e) {
      toast.error(`Could not add ${cook}`, { description: errorMessage(e) });
    }
  }
  return (
    <VirtualMenu<(typeof source.items)[number]>
      label="Your cooks outside this team"
      mode="action"
      source={source}
      onQuery={query.set}
      itemKey={(c) => c.name}
      isDisabled={() => !canManage}
      onSelect={(c) => void add(c.name)}
      empty={<p className="px-3 py-6 text-center text-[13px] text-muted-foreground">All your cooks are in {team}.</p>}
      renderItem={(c) => (
        <span className="flex min-h-9 items-center gap-2.5 text-[14px]">
          <Avatar name={c.name} className="rounded-lg opacity-60 grayscale" />
          <span className="min-w-0 flex-1">
            <span className="block truncate font-medium">{c.name}</span>
            <span className="block text-[11.5px] text-muted-foreground">{canManage ? `Not in ${team} yet: pick to add it` : `Not in ${team} yet`}</span>
          </span>
        </span>
      )}
      trigger={
        <button type="button" className={TOOL}>
          <UserPlus className="size-4 shrink-0" aria-hidden="true" /> Your cooks outside {team}
        </button>
      }
    />
  );
}
