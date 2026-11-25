import { useCallback, useMemo, useRef, useState } from "react";
import type { PassMessageView, WokenRef } from "@corntchap/api";
import { Check, CircleDashed, Cloud, Copy, Hash, Megaphone, MessageCircle, Reply, SquareArrowOutUpRight } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/api";
import { fmtClock, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { VirtualThread } from "@/components/ui/virtual-thread";
import { Avatar, IdChip, RowSkeleton } from "./bits";
import { Markdown } from "./markdown";
import { KindChip } from "./pass-bits";
import { StationLink, shortUrl } from "./station-link";
import { isEveryone, wakeState, wokenName, type CookInfo, type WakeSignals, type WakeState } from "./pass-data";

function dayLabel(iso: string, now: number): string {
  const d = new Date(iso);
  const t = new Date(now);
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(t) - start(d)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

/** A message with what the thread draws around it: the day heading above the first message of a day. */
interface Item {
  key: string;
  m: PassMessageView;
  first: boolean;
  last: boolean;
  day: string | null;
}

function buildItems(messages: PassMessageView[], now: number): Item[] {
  const out: Item[] = [];
  let prev: PassMessageView | null = null;
  const near = (a: PassMessageView, b: PassMessageView) =>
    (a.from ?? null) === (b.from ?? null) &&
    Math.abs(new Date(b.at).getTime() - new Date(a.at).getTime()) < 5 * 60_000 &&
    new Date(a.at).toDateString() === new Date(b.at).toDateString() &&
    isEveryone(a.body) === isEveryone(b.body);
  messages.forEach((m, i) => {
    const day = new Date(m.at).toDateString();
    const newDay = !prev || new Date(prev.at).toDateString() !== day;
    const next = messages[i + 1];
    out.push({ key: m.id, m, first: !prev || newDay || !near(prev, m), last: !next || !near(m, next), day: newDay ? dayLabel(m.at, now) : null });
    prev = m;
  });
  return out;
}

export interface ThreadActions {
  /** Prefill the composer with a quote. */
  reply: (text: string) => void;
  /** Open (or start) a direct conversation. */
  direct: (name: string) => void;
}

// ---------------------------------------------------------------- thread

export function Thread({
  potKey,
  messages,
  loading,
  error,
  me,
  who,
  cooks,
  chefNames,
  taskKeys,
  signals,
  actions,
  hasOlder,
  loadingOlder,
  onLoadOlder,
}: {
  potKey: string;
  messages: PassMessageView[] | undefined;
  loading: boolean;
  error: unknown;
  me: string;
  /** The other chef of a direct conversation, or null on the pass. */
  who: string | null;
  cooks: Map<string, CookInfo>;
  chefNames: Set<string>;
  taskKeys: string[];
  signals: WakeSignals;
  actions: ThreadActions;
  /** Older messages than the first one loaded exist on the server. */
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
}) {
  const now = useNow();
  const minute = Math.floor(now / 60_000);
  const last = messages?.[messages.length - 1]?.seq ?? 0;
  const items = useMemo(() => buildItems(messages ?? [], now), [messages, minute]); // eslint-disable-line react-hooks/exhaustive-deps
  const cook = who ? cooks.get(who) : undefined;
  const all = messages ?? [];

  const empty =
    loading && !messages ? (
      <div className="h-full overflow-y-auto px-3 py-3 md:px-6 md:py-4">
        <RowSkeleton rows={3} />
      </div>
    ) : error && !messages ? (
      <p className="py-6 text-center text-destructive">{errorMessage(error)}</p>
    ) : (
      <div className="h-full overflow-y-auto px-3 py-3 md:px-6 md:py-4">
        <QuietThread who={who} cook={cook} potKey={potKey} />
      </div>
    );

  return (
    <div role="log" aria-live="polite" aria-label="Messages" className="relative min-h-0 flex-1">
      <VirtualThread
        items={items}
        itemKey={(it) => it.key}
        lastSeq={last}
        hasOlder={hasOlder}
        loadingOlder={loadingOlder}
        onLoadOlder={onLoadOlder}
        empty={empty}
        itemClassName="mx-auto w-full max-w-3xl px-3 md:px-6"
        renderItem={(it) => (
          <Message
            m={it.m}
            day={it.day}
            first={it.first}
            last={it.last}
            mine={it.m.from === me}
            me={me}
            inDirect={who !== null}
            cooks={cooks}
            canDirect={!!it.m.from && it.m.from !== me && it.m.from !== who && chefNames.has(it.m.from)}
            taskKeys={taskKeys}
            messages={all}
            signals={signals}
            now={now}
            actions={actions}
          />
        )}
      />
    </div>
  );
}

function QuietThread({ who, cook, potKey }: { who: string | null; cook: CookInfo | undefined; potKey: string }) {
  let title: string;
  let body: string;
  let icon = <Hash className="size-5" aria-hidden="true" />;
  if (!who) {
    title = "A quiet channel";
    body = `Say something below. Every chef of ${potKey} reads it, and @name wakes a cook. It is kept a week.`;
  } else if (cook) {
    icon = <MessageCircle className="size-5" aria-hidden="true" />;
    title = `Nothing between you and ${who} yet`;
    body = cook.cloud
      ? `${who} runs in the cloud: each message starts a fresh run that answers once and stops. Put everything it needs in one message.`
      : `Say what you need; ${who} is woken with your message. Only the two of you read it.`;
  } else {
    icon = <MessageCircle className="size-5" aria-hidden="true" />;
    title = `Nothing between you and ${who} yet`;
    body = `Write below. Only the two of you read it.`;
  }
  return (
    <div className="grid min-h-full place-items-center px-4">
      <div className="max-w-xs text-center">
        <span className="mx-auto mb-3 grid size-11 place-items-center rounded-2xl bg-muted text-muted-foreground">{icon}</span>
        <p className="font-display text-[17px]">{title}</p>
        <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">{body}</p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- message

function useLongPress(onPress: () => void) {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const start = useRef<{ x: number; y: number } | null>(null);
  const cancel = useCallback(() => {
    clearTimeout(timer.current);
    start.current = null;
  }, []);
  return {
    onTouchStart: (e: React.TouchEvent) => {
      const t = e.touches[0];
      if (!t) return;
      start.current = { x: t.clientX, y: t.clientY };
      timer.current = setTimeout(() => {
        navigator.vibrate?.(8);
        onPress();
      }, 420);
    },
    onTouchMove: (e: React.TouchEvent) => {
      const t = e.touches[0];
      if (t && start.current && Math.hypot(t.clientX - start.current.x, t.clientY - start.current.y) > 8) cancel();
    },
    onTouchEnd: cancel,
    onTouchCancel: cancel,
    // a long press also raises the context menu on some browsers
    onContextMenu: (e: React.MouseEvent) => {
      if (window.matchMedia("(pointer: coarse)").matches) {
        e.preventDefault();
        onPress();
      }
    },
  };
}

function quoteOf(m: PassMessageView, name: string): string {
  const lines = m.body
    .replace(/^>.*$/gm, "")
    .trim()
    .split("\n")
    .slice(0, 6)
    .join("\n")
    .slice(0, 400);
  return `> ${name}: ${lines.split("\n").join("\n> ")}\n\n`;
}

function Message({
  m,
  day,
  first,
  last,
  mine,
  me,
  inDirect,
  cooks,
  canDirect,
  taskKeys,
  messages,
  signals,
  now,
  actions,
}: {
  m: PassMessageView;
  day: string | null;
  first: boolean;
  last: boolean;
  mine: boolean;
  me: string;
  inDirect: boolean;
  cooks: Map<string, CookInfo>;
  canDirect: boolean;
  taskKeys: string[];
  messages: PassMessageView[];
  signals: WakeSignals;
  now: number;
  actions: ThreadActions;
}) {
  const openTask = useOpenTask();
  const [sheet, setSheet] = useState(false);
  const press = useLongPress(() => setSheet(true));
  const everyone = isEveryone(m.body);
  const name = mine ? "you" : (m.from ?? "the kitchen");
  const cook = m.from ? cooks.get(m.from) : undefined;
  const mentionsMe = !mine && new RegExp(`(^|\\s)@${me.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "i").test(m.body);
  const taskId =
    m.task ??
    (taskKeys.length
      ? new RegExp(`(?:^|[^\\w-])((?:${taskKeys.join("|")})-\\d+)\\b`).exec(m.body)?.[1]
      : undefined) ??
    null;

  const list: { id: string; label: string; icon: React.ReactNode; run: () => void }[] = [
    {
      id: "copy",
      label: "Copy text",
      icon: <Copy />,
      run: () => {
        void navigator.clipboard?.writeText(m.body).then(
          () => toast("Copied"),
          () => toast.error("Could not copy"),
        );
      },
    },
    { id: "reply", label: "Reply with a quote", icon: <Reply />, run: () => actions.reply(quoteOf(m, name)) },
  ];
  if (taskId) list.push({ id: "task", label: `Open ${taskId}`, icon: <SquareArrowOutUpRight />, run: () => openTask(taskId) });
  if (canDirect && m.from) list.push({ id: "dm", label: `Message ${m.from} directly`, icon: <MessageCircle />, run: () => actions.direct(m.from!) });

  const woken = dedupeWoken(m.woken);

  return (
    <>
      {day && (
        <div className="flex justify-center pt-3 pb-1" role="separator" aria-label={day}>
          <span className="rounded-full bg-muted px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground">{day}</span>
        </div>
      )}
      <div className={cn("group/msg relative flex min-w-0 gap-2", mine && "flex-row-reverse", first ? "pt-3.5" : "pt-0.5", day && "pt-1")}>
      {!mine && <span className="w-8 shrink-0">{first && <Avatar name={m.from ?? "the kitchen"} className="size-8 rounded-xl text-[13px]" />}</span>}
      <div className={cn("flex min-w-0 max-w-[88%] flex-col md:max-w-[75%]", mine ? "items-end" : "items-start")}>
        {first && !mine && (
          <p className="mb-0.5 flex max-w-full min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 px-1 text-[12.5px]">
            <span className="max-w-full truncate font-semibold" title={m.from ?? undefined}>
              {name}
            </span>
            {m.from_kind === "agent" && !inDirect && <KindChip>{cook?.agentKind ?? "cook"}</KindChip>}
            {m.station_label && (
              <span className="max-w-[12rem] truncate font-mono text-[10.5px] text-muted-foreground" title={`Session ${m.station}: ${m.station_label}`}>
                <StationLink label={m.station_url ? shortUrl(m.station_url) : m.station_label} url={m.station_url} title={m.station_url ?? undefined} />
              </span>
            )}
            {m.via && (
              <span className="max-w-full truncate text-[11.5px] text-muted-foreground" title={`${m.via} posted it for ${m.from ?? "its owner"}`}>
                via {m.via}
              </span>
            )}
          </p>
        )}
        <div className={cn("flex max-w-full min-w-0 items-end gap-2", mine && "flex-row-reverse")}>
          <div
            {...press}
            className={cn(
              "relative min-w-0 max-w-full rounded-[18px] px-3.5 py-2 [-webkit-touch-callout:none] pointer-coarse:select-none",
              mine ? "bg-primary/12 dark:bg-primary/22" : "bg-muted",
              mine ? (first ? "rounded-tr-md" : last ? "" : "rounded-r-md") : first ? "rounded-tl-md" : last ? "" : "rounded-l-md",
              mine && !first && "rounded-tr-md",
              !mine && !first && "rounded-tl-md",
              mentionsMe && !everyone && "ring-1 ring-primary/40",
            )}
          >
            {(everyone || m.to_station_label || (m.task && !m.body.includes(m.task))) && (
              <p className="mb-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground">
                {everyone && (
                  <span className="inline-flex items-center gap-1 font-medium text-primary">
                    <Megaphone className="size-3" aria-hidden="true" /> to everyone
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
                    className="rounded-md border border-border bg-background/70 px-1.5 py-px hover:border-foreground/40"
                    title={`Open ${m.task}`}
                  >
                    <IdChip id={m.task} className="text-[11px]" />
                  </button>
                )}
              </p>
            )}
            <Markdown
              mentions={me}
              taskKeys={taskKeys}
              onTask={openTask}
              className="min-w-0 text-[14.5px] leading-[1.5] break-words md:text-[14px] [&_blockquote]:text-[13px] [&_code]:bg-background/60 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_pre]:bg-background/70 [&_pre]:text-[12.5px] [&_table]:block [&_table]:overflow-x-auto"
            >
              {m.body}
            </Markdown>
            {/* desktop: the actions float over the bubble's corner while the pointer is on the message */}
            <div
              className={cn(
                "absolute -top-3.5 z-10 hidden items-center gap-px rounded-lg border border-border bg-card p-0.5 shadow-sm pointer-fine:flex",
                "pointer-events-none opacity-0 transition-opacity group-focus-within/msg:pointer-events-auto group-focus-within/msg:opacity-100 group-hover/msg:pointer-events-auto group-hover/msg:opacity-100",
                mine ? "left-2" : "right-2",
              )}
            >
              {list.map((a) => (
                <button
                  key={a.id}
                  type="button"
                  onClick={a.run}
                  title={a.label}
                  aria-label={a.label}
                  className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:bg-muted [&_svg]:size-3.5"
                >
                  {a.icon}
                </button>
              ))}
            </div>
          </div>
          <span
            className="mb-1 shrink-0 font-mono text-[10.5px] text-muted-foreground opacity-0 transition-opacity group-hover/msg:opacity-100 pointer-coarse:hidden"
            title={new Date(m.at).toLocaleString()}
          >
            {fmtClock(m.at)}
          </span>
        </div>
        {woken.length > 0 && <WakeChips m={m} woken={woken} messages={messages} signals={signals} now={now} cooks={cooks} />}
        {last && (
          <p className="mt-0.5 hidden px-1 font-mono text-[10.5px] text-muted-foreground pointer-coarse:block" title={new Date(m.at).toLocaleString()}>
            {fmtClock(m.at)}
          </p>
        )}
      </div>

      <Sheet open={sheet} onOpenChange={setSheet}>
        <SheetContent side="bottom" className="gap-0 px-3 pb-[calc(0.75rem+env(safe-area-inset-bottom))]">
          <SheetTitle className="sr-only">Message actions</SheetTitle>
          <SheetDescription className="sr-only">Copy, reply, open the task or write to {m.from ?? "the sender"} directly</SheetDescription>
          <p className="mx-1 mt-1 mb-2 line-clamp-2 rounded-xl bg-muted px-3 py-2 text-[13px] text-muted-foreground">{m.body}</p>
          <ul>
            {list.map((a) => (
              <li key={a.id}>
                <button
                  type="button"
                  onClick={() => {
                    setSheet(false);
                    a.run();
                  }}
                  className="flex min-h-12 w-full items-center gap-3 rounded-xl px-3 text-left text-[15px] active:bg-muted [&_svg]:size-4.5 [&_svg]:text-muted-foreground"
                >
                  {a.icon}
                  {a.label}
                </button>
              </li>
            ))}
          </ul>
        </SheetContent>
      </Sheet>
    </div>
    </>
  );
}

// ---------------------------------------------------------------- wake status

/** One chip per cook: a cook woken on two sessions is one chip. */
function dedupeWoken(w: WokenRef[]): WokenRef[] {
  const seen = new Map<string, WokenRef>();
  for (const x of w) if (!seen.has(x.cook)) seen.set(x.cook, x);
  return [...seen.values()];
}

const WAKE_LABEL: Record<WakeState, string> = {
  woken: "woken",
  working: "working…",
  replied: "replied",
  quiet: "no reply yet",
};

function WakeChips({
  m,
  woken,
  messages,
  signals,
  now,
  cooks,
}: {
  m: PassMessageView;
  woken: WokenRef[];
  messages: PassMessageView[];
  signals: WakeSignals;
  now: number;
  cooks: Map<string, CookInfo>;
}) {
  return (
    <ul className="mt-1 flex max-w-full flex-wrap gap-1 px-0.5" aria-label="Who this woke">
      {woken.map((w) => {
        const st = wakeState(m, w, messages, signals, now);
        const cloud = w.cloud || cooks.get(w.cook)?.cloud;
        const sessions = m.woken.filter((x) => x.cook === w.cook);
        return (
          <li
            key={w.cook}
            title={sessions.map(wokenName).join("\n")}
            className={cn(
              "inline-flex max-w-full items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] leading-4",
              st === "working" ? "border-corn/50 bg-corn/12 text-foreground" : "border-border bg-background/60 text-muted-foreground",
              st === "replied" && "text-success dark:text-success",
            )}
          >
            <WakeMark state={st} />
            <span className="truncate">
              <span className="font-medium text-foreground">{w.cook}</span>
              <span className="sr-only">: </span> <span aria-live="polite">{WAKE_LABEL[st]}</span>
            </span>
            {cloud && (
              <span className="inline-flex shrink-0 items-center gap-1 border-l border-border pl-1.5 text-muted-foreground">
                <Cloud className="size-3" aria-hidden="true" /> cloud run · fire and forget
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function WakeMark({ state }: { state: WakeState }) {
  if (state === "replied") return <Check className="size-3 shrink-0" aria-hidden="true" />;
  if (state === "quiet") return <CircleDashed className="size-3 shrink-0" aria-hidden="true" />;
  return (
    <span
      aria-hidden="true"
      className={cn("size-2 shrink-0 rounded-full", state === "working" ? "live-dot bg-corn" : "border border-muted-foreground/60")}
    />
  );
}
