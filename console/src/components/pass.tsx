import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import type { PassMessageView, PassPage, WokenRef } from "@corntchap/api";
import { ArrowDown, ChevronDown, Hash, Loader2, Megaphone, MessageSquarePlus, Plus, Send } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { client, errorMessage } from "@/lib/api";
import { useInvalidateAll, useProc, useProjects, useTeamChefs, type MemberRow } from "@/lib/data";
import { fmtClock, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { useSession } from "@/lib/session";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Avatar, EmptyState, IdChip, RowSkeleton } from "./bits";
import { Markdown } from "./markdown";
import { RefreshButton } from "./refresh-button";

const POT = "pot";

/** "doug · quiet-planet" for a woken session, "doug" for a cook woken through its wake target. */
function wokenName(w: WokenRef): string {
  const where = w.label ?? w.station;
  const name = where ? `${w.cook} · ${where}` : w.cook;
  // a cloud cook answers once in a fresh run, then stops
  return w.cloud ? `${name} (cloud run, fire and forget)` : name;
}

/**
 * One conversation of a pot's pass, newest 100 messages. Reading marks them read on the server, so the pot
 * summaries (the unread badges) are refreshed after each read. Lives under ["data"], so `pass.posted` live
 * events (which invalidate everything) refetch it.
 */
function usePass(pot: string, withWho: string) {
  const projects = useProjects();
  const refetchProjects = useRef(projects.refetch);
  refetchProjects.current = projects.refetch;
  const q = useQuery({
    queryKey: ["data", "pass", pot, withWho],
    queryFn: async (): Promise<PassPage> => {
      const page = await client.procedures.passRead({ project: pot, chef: withWho, limit: 100 });
      if (page.messages.length > 0) void refetchProjects.current();
      return page;
    },
    staleTime: 1500,
    placeholderData: (prev) => prev,
  });
  return { page: q.data, isLoading: q.isLoading, error: q.error as unknown };
}

const isEveryone = (body: string) => /(^|\s)@(all|everyone)\b/i.test(body);

function dayLabel(iso: string, now: number): string {
  const d = new Date(iso);
  const t = new Date(now);
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(t) - start(d)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

// ---------------------------------------------------------------- the view

export function PassView({ potKey }: { potKey: string }) {
  const me = useSession();
  const [params, setParams] = useSearchParams();
  const [switcher, setSwitcher] = useState(false);
  const projects = useProjects();
  const team = projects.data?.find((p) => p.key === potKey)?.team ?? null;
  const members = useTeamChefs(team);
  const chefs = useMemo(() => (members.data ?? []).filter((m) => m.name !== me.name), [members.data, me.name]);

  const withWho = params.get("with") || POT;
  const select = (w: string) => {
    setSwitcher(false);
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

  const { page, isLoading, error } = usePass(potKey, withWho);
  const unreadDirect = useMemo(() => {
    const m = new Map<string, number>();
    for (const d of page?.unread.direct ?? []) if (d.chef) m.set(d.chef, d.count);
    return m;
  }, [page]);
  const unreadPot = page?.unread.pot ?? 0;

  // Every direct conversation you are in, the most recent first (the server lists them all).
  const direct = useMemo(() => {
    const names = [...unreadDirect.keys()];
    if (withWho !== POT && !names.includes(withWho)) names.unshift(withWho);
    return names.filter((n) => n !== me.name);
  }, [unreadDirect, withWho, me.name]);
  const others = useMemo(() => chefs.filter((c) => !direct.includes(c.name)), [chefs, direct]);
  const kindOf = (name: string) => members.data?.find((m) => m.name === name)?.kind;
  const unreadElsewhere =
    (withWho === POT ? 0 : unreadPot) + [...unreadDirect.entries()].reduce((n, [k, c]) => n + (k === withWho ? 0 : c), 0);

  const list = (
    <ConvList
      withWho={withWho}
      select={select}
      unreadPot={unreadPot}
      unreadDirect={unreadDirect}
      direct={direct}
      others={others}
      kindOf={kindOf}
    />
  );

  return (
    <div
      className={cn(
        "flex min-w-0 flex-col bg-background",
        // phones: a real chat screen between the top bar and the tab bar
        "max-md:fixed max-md:inset-x-0 max-md:top-[calc(3.5rem+1px+env(safe-area-inset-top))] max-md:bottom-[calc(4rem+env(safe-area-inset-bottom))] max-md:z-20",
        // tablet and up: conversations left, thread right
        "md:h-[min(78dvh,800px)] md:min-h-[30rem] md:flex-row md:overflow-hidden md:rounded-[14px] md:border md:border-border md:bg-card",
      )}
    >
      <aside aria-label="Conversations" className="scrollbar-thin hidden w-60 shrink-0 overflow-y-auto border-r border-border bg-muted/30 p-2 md:block lg:w-64">
        {list}
      </aside>

      <section aria-label={withWho === POT ? "The pass" : `Conversation with ${withWho}`} className="flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex min-h-14 items-center gap-1 border-b border-border bg-card px-2 md:px-4">
          <button
            type="button"
            onClick={() => setSwitcher(true)}
            aria-label="Switch conversation"
            className="relative flex min-h-12 min-w-0 flex-1 items-center gap-2.5 rounded-xl px-2 text-left active:bg-muted md:pointer-events-none md:min-h-0 md:px-0"
          >
            {withWho === POT ? (
              <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-corn text-corn-foreground md:size-8">
                <Hash className="size-4" aria-hidden="true" />
              </span>
            ) : (
              <Avatar name={withWho} className="size-9 rounded-xl text-[14px] md:size-8" />
            )}
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5 truncate font-display text-[17px] leading-tight">
                <span className="truncate">{withWho === POT ? "pass" : withWho}</span>
                {withWho !== POT && kindOf(withWho) === "agent" && <span className="label-caps shrink-0">cook</span>}
                <ChevronDown className="size-4 shrink-0 text-muted-foreground md:hidden" aria-hidden="true" />
              </span>
              <span className="block truncate text-[12px] text-muted-foreground">
                {withWho === POT ? `Every chef of ${potKey} reads this` : `Only you and ${withWho} read this`}
              </span>
            </span>
            {unreadElsewhere > 0 && (
              <span className="absolute top-2 left-9 size-2.5 rounded-full bg-primary ring-2 ring-card md:hidden">
                <span className="sr-only">{unreadElsewhere} unread elsewhere</span>
              </span>
            )}
          </button>
          <RefreshButton size="icon-sm" />
        </header>

        <Thread
          key={`${potKey}/${withWho}`}
          messages={page?.messages}
          loading={isLoading}
          error={error}
          me={me.name}
          direct={withWho !== POT}
          who={withWho}
        />
        <Composer potKey={potKey} withWho={withWho} chefs={chefs} />
      </section>

      <Sheet open={switcher} onOpenChange={setSwitcher}>
        <SheetContent side="bottom" className="max-h-[80dvh] gap-0 px-3 pb-3 md:hidden">
          <SheetTitle className="px-2 pb-2 font-display text-[18px]">Conversations</SheetTitle>
          <SheetDescription className="sr-only">Pick the pass, or a chef to write to directly</SheetDescription>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">{list}</div>
        </SheetContent>
      </Sheet>
    </div>
  );
}

function ConvList({
  withWho,
  select,
  unreadPot,
  unreadDirect,
  direct,
  others,
  kindOf,
}: {
  withWho: string;
  select: (w: string) => void;
  unreadPot: number;
  unreadDirect: Map<string, number>;
  direct: string[];
  others: MemberRow[];
  kindOf: (n: string) => string | undefined;
}) {
  return (
    <ul className="space-y-0.5">
      <ConvRow active={withWho === POT} onClick={() => select(POT)} unread={withWho === POT ? 0 : unreadPot}>
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-corn text-corn-foreground">
          <Hash className="size-4" aria-hidden="true" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">pass</span>
          <span className="block truncate text-[11.5px] text-muted-foreground">Everyone in the pot</span>
        </span>
      </ConvRow>
      {direct.length > 0 && <li className="label-caps list-none px-2.5 pt-3 pb-1">Direct</li>}
      {direct.map((n) => (
        <ConvRow key={n} active={withWho === n} onClick={() => select(n)} unread={withWho === n ? 0 : (unreadDirect.get(n) ?? 0)}>
          <Avatar name={n} className="size-8 text-[13px]" />
          <span className="min-w-0 flex-1 truncate font-medium">{n}</span>
          {kindOf(n) === "agent" && <span className="label-caps shrink-0">cook</span>}
        </ConvRow>
      ))}
      {others.length > 0 && (
        <li className="label-caps flex list-none items-center gap-1.5 px-2.5 pt-3 pb-1">
          <MessageSquarePlus className="size-3" aria-hidden="true" /> Start a conversation
        </li>
      )}
      {others.map((c) => (
        <ConvRow key={c.name} active={false} onClick={() => select(c.name)} unread={0}>
          <Avatar name={c.name} className="size-8 text-[13px] opacity-80" />
          <span className="min-w-0 flex-1 truncate text-muted-foreground">{c.name}</span>
          {c.kind === "agent" && <span className="label-caps shrink-0">cook</span>}
        </ConvRow>
      ))}
    </ul>
  );
}

function ConvRow({ active, unread, onClick, children }: { active: boolean; unread: number; onClick: () => void; children: React.ReactNode }) {
  return (
    <li className="list-none">
      <button
        type="button"
        onClick={onClick}
        aria-current={active ? "true" : undefined}
        className={cn(
          "flex min-h-12 w-full items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left text-[14px] transition-colors hover:bg-muted active:bg-muted",
          active && "bg-muted",
        )}
      >
        {children}
        {unread > 0 && (
          <span className="inline-flex min-w-5 shrink-0 items-center justify-center rounded-full bg-primary px-1.5 font-mono text-[11px] leading-5 font-bold text-primary-foreground">
            <span aria-hidden="true">{unread}</span>
            <span className="sr-only"> unread</span>
          </span>
        )}
      </button>
    </li>
  );
}

// ---------------------------------------------------------------- thread

type Item = { kind: "day"; key: string; label: string } | { kind: "msg"; key: string; m: PassMessageView; first: boolean; last: boolean };

function buildItems(messages: PassMessageView[], now: number): Item[] {
  const out: Item[] = [];
  let prev: PassMessageView | null = null;
  messages.forEach((m, i) => {
    const day = new Date(m.at).toDateString();
    const newDay = !prev || new Date(prev.at).toDateString() !== day;
    if (newDay) out.push({ kind: "day", key: `d-${day}`, label: dayLabel(m.at, now) });
    const same =
      !newDay &&
      prev !== null &&
      (prev.from ?? null) === (m.from ?? null) &&
      new Date(m.at).getTime() - new Date(prev.at).getTime() < 5 * 60_000 &&
      isEveryone(prev.body) === isEveryone(m.body);
    const next = messages[i + 1];
    const nextSame =
      !!next &&
      new Date(next.at).toDateString() === day &&
      (next.from ?? null) === (m.from ?? null) &&
      new Date(next.at).getTime() - new Date(m.at).getTime() < 5 * 60_000 &&
      isEveryone(next.body) === isEveryone(m.body);
    out.push({ kind: "msg", key: m.id, m, first: !same, last: !nextSame });
    prev = m;
  });
  return out;
}

function Thread({
  messages,
  loading,
  error,
  me,
  direct,
  who,
}: {
  messages: PassMessageView[] | undefined;
  loading: boolean;
  error: unknown;
  me: string;
  direct: boolean;
  who: string;
}) {
  const now = useNow();
  const openTask = useOpenTask();
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [away, setAway] = useState(false);
  const last = messages?.[messages.length - 1]?.seq ?? 0;
  const items = useMemo(() => buildItems(messages ?? [], now), [messages, now]);

  // Follow the newest message while you are at the bottom; when you have scrolled up to read, stay put.
  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [last, loading]);
  useEffect(() => {
    if (!stick.current) setAway(true);
  }, [last]);

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 64;
          if (stick.current) setAway(false);
        }}
        role="log"
        aria-live="polite"
        className="scrollbar-thin h-full overflow-x-hidden overflow-y-auto overscroll-contain px-3 py-3 md:px-5"
      >
        {loading && !messages ? (
          <RowSkeleton rows={3} />
        ) : error && !messages ? (
          <p className="py-6 text-center text-destructive">{errorMessage(error)}</p>
        ) : !messages || messages.length === 0 ? (
          <div className="grid min-h-full place-items-center">
            <EmptyState band={false} title={direct ? `Nothing between you and ${who} yet` : "The pass is quiet"} className="border-0 bg-transparent">
              {direct ? "Write below. Only the two of you read it." : "Say something below. Every chef of the pot reads it; it is kept a week."}
            </EmptyState>
          </div>
        ) : (
          <ul className="flex min-h-full flex-col justify-end">
            {items.map((it) =>
              it.kind === "day" ? (
                <li key={it.key} className="my-3 flex items-center gap-3 first:mt-0" aria-label={it.label}>
                  <span className="h-px flex-1 bg-border" />
                  <span className="label-caps">{it.label}</span>
                  <span className="h-px flex-1 bg-border" />
                </li>
              ) : (
                <Message key={it.key} m={it.m} first={it.first} last={it.last} mine={it.m.from === me} me={me} openTask={openTask} />
              ),
            )}
          </ul>
        )}
      </div>
      {away && (
        <button
          type="button"
          onClick={() => {
            const el = box.current;
            if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
          }}
          className="absolute right-4 bottom-3 inline-flex h-9 items-center gap-1.5 rounded-full border border-border bg-card px-3 text-[12.5px] font-medium shadow-md"
        >
          <ArrowDown className="size-3.5" aria-hidden="true" /> Newer
        </button>
      )}
    </div>
  );
}

function Message({
  m,
  first,
  last,
  mine,
  me,
  openTask,
}: {
  m: PassMessageView;
  first: boolean;
  last: boolean;
  mine: boolean;
  me: string;
  openTask: (id: string) => void;
}) {
  const everyone = isEveryone(m.body);
  const name = mine ? "you" : (m.from ?? "the kitchen");
  const mentionsMe = !mine && new RegExp(`(^|\\s)@${me.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "i").test(m.body);
  return (
    <li className={cn("flex min-w-0 gap-2", mine && "flex-row-reverse", first ? "mt-3 first:mt-0" : "mt-0.5")}>
      {!mine && (
        <span className="w-7 shrink-0">
          {last && <Avatar name={m.from ?? "the kitchen"} className="size-7 rounded-lg text-[12px]" />}
        </span>
      )}
      <div className={cn("flex min-w-0 max-w-[88%] flex-col md:max-w-[78%]", mine ? "items-end" : "items-start")}>
        {first && !mine && (
          <p className="mb-0.5 flex max-w-full min-w-0 flex-wrap items-baseline gap-x-1.5 px-1 text-[12.5px]">
            <span className="max-w-full truncate font-semibold" title={m.from ?? undefined}>
              {name}
            </span>
            {m.from_kind === "agent" && <span className="label-caps">cook</span>}
            {m.station_label && (
              <span className="max-w-[12rem] truncate font-mono text-[10.5px] text-muted-foreground" title={`Session ${m.station}: ${m.station_label}`}>
                · {m.station_label}
              </span>
            )}
            {m.via && (
              <span className="max-w-full truncate text-[11.5px] text-muted-foreground" title={`${m.via} posted it for ${m.from ?? "its owner"}`}>
                via {m.via}
              </span>
            )}
          </p>
        )}
        <div
          className={cn(
            "min-w-0 max-w-full rounded-2xl border px-3 py-1.5",
            mine ? "border-primary/25 bg-primary/10" : "border-border bg-card",
            mine ? (first ? "rounded-tr-md" : "") : first ? "rounded-tl-md" : "",
            everyone && "border-l-[3px] border-l-primary",
            mentionsMe && !everyone && "ring-1 ring-primary/40",
          )}
        >
          {(everyone || m.to_station_label || m.task) && (
            <p className="mb-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground">
              {everyone && (
                <span className="inline-flex items-center gap-1 font-mono tracking-wide uppercase">
                  <Megaphone className="size-3" aria-hidden="true" /> everyone
                </span>
              )}
              {m.to_station_label && (
                <span className="max-w-[12rem] truncate font-mono" title={`To session ${m.to_station}`}>
                  → {m.to_station_label}
                </span>
              )}
              {m.task && (
                <button
                  type="button"
                  onClick={() => openTask(m.task!)}
                  className="rounded-md border border-border bg-muted px-1.5 py-px hover:border-foreground/40"
                  title={`Open ${m.task}`}
                >
                  <IdChip id={m.task} className="text-[11px]" />
                </button>
              )}
            </p>
          )}
          <Markdown
            mentions={me}
            className="min-w-0 text-[14px] leading-[1.5] break-words [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_table]:block [&_table]:overflow-x-auto"
          >
            {m.body}
          </Markdown>
          {last && (
            <p className={cn("mt-0.5 flex justify-end font-mono text-[10px] text-muted-foreground")} title={new Date(m.at).toLocaleString()}>
              {fmtClock(m.at)}
            </p>
          )}
        </div>
        {last && m.woken.length > 0 && (
          <p className="mt-0.5 max-w-full px-1 text-[11px] leading-snug text-muted-foreground" title={m.woken.map(wokenName).join("\n")}>
            woke {m.woken.length === 1 ? wokenName(m.woken[0]!) : `${m.woken.length} sessions: ${m.woken.map(wokenName).join(", ")}`}
          </p>
        )}
      </div>
    </li>
  );
}

// ---------------------------------------------------------------- composer

/** Write on the pass (or to one chef). Enter sends on desktop, Shift+Enter is a new line; `@` suggests chefs. */
function Composer({ potKey, withWho, chefs }: { potKey: string; withWho: string; chefs: MemberRow[] }) {
  const invalidate = useInvalidateAll();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [caret, setCaret] = useState(0);
  const [pick, setPick] = useState(0);
  const [everyone, setEveryone] = useState(false);
  const [session, setSession] = useState("");
  const ta = useRef<HTMLTextAreaElement>(null);
  const channel = withWho === POT;
  // a direct message to a cook can go to one of its sessions (stations) rather than its main one
  const toCook = !channel && chefs.find((c) => c.name === withWho)?.kind === "agent";
  const stations = useProc("stationList", { project: potKey, cook: withWho }, { enabled: toCook });
  const stationList = stations.data ?? [];
  const hasOptions = channel || (toCook && stationList.length > 1);

  // grow with the text, up to about six lines
  useLayoutEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  const mention = useMemo(() => {
    const m = /(^|\s)@([\w.-]*)$/.exec(text.slice(0, caret));
    return m ? { start: caret - m[2]!.length - 1, q: m[2]!.toLowerCase() } : null;
  }, [text, caret]);
  const options = useMemo(
    () => (mention ? chefs.filter((c) => c.name.toLowerCase().startsWith(mention.q)).slice(0, 6) : []),
    [mention, chefs],
  );

  function insert(name: string) {
    if (!mention) return;
    const next = `${text.slice(0, mention.start)}@${name} ${text.slice(caret)}`;
    const pos = mention.start + name.length + 2;
    setText(next);
    setCaret(pos);
    setPick(0);
    requestAnimationFrame(() => {
      ta.current?.focus();
      ta.current?.setSelectionRange(pos, pos);
    });
  }

  async function send() {
    const body = text.trim();
    if (!body || busy) return;
    setBusy(true);
    setText("");
    try {
      const sent = await client.procedures.passPost({
        project: potKey,
        message: body,
        to: channel ? null : session || withWho,
        task: null,
        fromOwner: null,
        broadcast: channel && everyone ? true : null,
        station: null,
      });
      void invalidate();
      if (sent.woken.length > 0)
        toast(`Woke ${sent.woken.length === 1 ? wokenName(sent.woken[0]!) : `${sent.woken.length} sessions`}`, {
          ...(sent.note ? { description: sent.note } : {}),
        });
    } catch (e) {
      setText(body);
      toast.error("Could not send it", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  const isTouch = () => window.matchMedia("(pointer: coarse)").matches;

  return (
    <div className="relative border-t border-border bg-card px-2.5 pt-2 pb-2.5 md:px-4">
      {options.length > 0 && (
        <ul role="listbox" aria-label="Mention a chef" className="absolute right-3 bottom-full left-3 mb-1 rounded-xl border border-border bg-popover p-1 shadow-md">
          {options.map((c, i) => (
            <li key={c.name} role="option" aria-selected={i === pick}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  insert(c.name);
                }}
                className={cn("flex min-h-11 w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[14px]", i === pick && "bg-muted")}
              >
                <Avatar name={c.name} />
                <span className="min-w-0 flex-1 truncate">{c.name}</span>
                {c.kind === "agent" && <span className="label-caps shrink-0">cook</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
      {(everyone && channel) || session ? (
        <p className="mb-1.5 flex flex-wrap gap-x-3 px-1 text-[11.5px] text-muted-foreground">
          {everyone && channel && (
            <span className="inline-flex items-center gap-1 font-medium text-primary">
              <Megaphone className="size-3" aria-hidden="true" /> Waking every session
            </span>
          )}
          {session && toCook && <span className="font-mono">to session {stationList.find((s) => s.id === session)?.label ?? session}</span>}
        </p>
      ) : null}
      <div className="flex items-end gap-1.5">
        {hasOptions && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button type="button" variant="ghost" size="icon" aria-label="Message options" className="shrink-0 text-muted-foreground">
                <Plus />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="min-w-56">
              {channel ? (
                <DropdownMenuCheckboxItem checked={everyone} onCheckedChange={setEveryone} className="min-h-10">
                  <span className="min-w-0">
                    Wake everyone
                    <span className="block text-[11.5px] text-muted-foreground">Every session of every cook (same as @all)</span>
                  </span>
                </DropdownMenuCheckboxItem>
              ) : (
                <>
                  <DropdownMenuLabel className="label-caps py-1">To session</DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  <DropdownMenuRadioGroup value={session} onValueChange={setSession}>
                    <DropdownMenuRadioItem value="" className="min-h-10">
                      its main session
                    </DropdownMenuRadioItem>
                    {stationList.map((st) => (
                      <DropdownMenuRadioItem key={st.id} value={st.id} className="min-h-10">
                        <span className="truncate">{(st.label ?? st.directory ?? st.session) + (st.main ? " (main)" : "")}</span>
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
        <Textarea
          ref={ta}
          rows={1}
          value={text}
          aria-label={channel ? "Write on the pass" : `Write to ${withWho}`}
          placeholder={channel ? "Write to the pass…" : `Write to ${withWho}…`}
          className="max-h-40 min-h-11 flex-1 resize-none rounded-2xl py-2.5 leading-snug"
          onChange={(e) => {
            setText(e.target.value);
            setCaret(e.target.selectionStart ?? e.target.value.length);
            setPick(0);
          }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onKeyDown={(e) => {
            if (options.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                setPick((p) => (p + (e.key === "ArrowDown" ? 1 : options.length - 1)) % options.length);
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                insert(options[pick]!.name);
                return;
              }
            }
            // on a phone Enter is a new line (the send button sends); with a keyboard Enter sends
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && !isTouch()) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <Button
          type="button"
          size="icon"
          onClick={() => void send()}
          disabled={busy || !text.trim()}
          aria-label="Send"
          className="shrink-0 rounded-full"
        >
          {busy ? <Loader2 className="animate-spin" /> : <Send />}
        </Button>
      </div>
      <p className="mt-1.5 hidden px-1 font-mono text-[10.5px] text-muted-foreground md:block">
        ↵ to send · shift+↵ for a new line · markdown ok · {channel ? "@name wakes a cook, @all every session" : "a message to a cook wakes it"} · kept a week
      </p>
    </div>
  );
}
