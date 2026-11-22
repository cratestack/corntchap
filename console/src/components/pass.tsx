import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import type { PassMessageView, PassPage, WokenRef } from "@corntchap/api";
import { Hash, Loader2, Send } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { client, errorMessage } from "@/lib/api";
import { useInvalidateAll, useProjects, useTeamChefs, type MemberRow } from "@/lib/data";
import { fmtAgo, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { useSession } from "@/lib/session";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Avatar, EmptyState, IdChip, RowSkeleton } from "./bits";
import { Markdown } from "./markdown";
import { RefreshButton } from "./refresh-button";

const POT = "pot";

/** "doug · quiet-planet" for a woken session, "doug" for a cook woken through its wake target. */
function wokenName(w: WokenRef): string {
  const where = w.label ?? w.station;
  return where ? `${w.cook} · ${where}` : w.cook;
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

export function PassView({ potKey }: { potKey: string }) {
  const me = useSession();
  const [params, setParams] = useSearchParams();
  const projects = useProjects();
  const team = projects.data?.find((p) => p.key === potKey)?.team ?? null;
  const members = useTeamChefs(team);
  const chefs = useMemo(() => (members.data ?? []).filter((m) => m.name !== me.name), [members.data, me.name]);

  const withWho = params.get("with") || POT;
  const select = (w: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (w === POT) n.delete("with");
        else n.set("with", w);
        return n;
      },
      { replace: true },
    );

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
  const kindOf = (name: string) => members.data?.find((m) => m.name === name)?.kind;

  return (
    <div className="grid min-w-0 gap-4 lg:grid-cols-[15rem_minmax(0,1fr)]">
      <aside aria-label="Conversations" className="min-w-0 rounded-[14px] border border-border bg-card p-2 lg:self-start">
        <ul className="scrollbar-thin max-h-44 space-y-0.5 overflow-y-auto lg:max-h-[min(60dvh,520px)]">
          <ConvRow active={withWho === POT} onClick={() => select(POT)} unread={withWho === POT ? 0 : unreadPot}>
            <span className="grid size-6 shrink-0 place-items-center rounded-md bg-corn text-corn-foreground">
              <Hash className="size-3.5" aria-hidden="true" />
            </span>
            <span className="min-w-0 flex-1 truncate font-medium">pass</span>
          </ConvRow>
          {direct.length > 0 && <li className="label-caps px-2.5 pt-2.5 pb-1">Direct</li>}
          {direct.map((n) => (
            <ConvRow key={n} active={withWho === n} onClick={() => select(n)} unread={withWho === n ? 0 : (unreadDirect.get(n) ?? 0)}>
                <Avatar name={n} />
                <span className="min-w-0 flex-1 truncate font-medium" title={n}>
                  {n}
                </span>
                {kindOf(n) === "agent" && <span className="label-caps shrink-0">cook</span>}
            </ConvRow>
          ))}
        </ul>
        {chefs.length > 0 && <NewDirect chefs={chefs} onPick={select} />}
      </aside>

      <section aria-label={withWho === POT ? "The pass" : `Conversation with ${withWho}`} className="flex min-w-0 flex-col overflow-hidden rounded-[14px] border border-border bg-card">
        <header className="flex items-center gap-2 border-b border-border px-4 py-2.5">
          <div className="min-w-0 flex-1">
            <h2 className="flex items-center gap-1.5 truncate font-display text-[17px] leading-tight">
              {withWho === POT ? (
                <>
                  <Hash className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  pass
                </>
              ) : (
                <span className="truncate">{withWho}</span>
              )}
            </h2>
            <p className="truncate text-[12px] text-muted-foreground">
              {withWho === POT ? `Every chef of ${potKey} reads this.` : `Only you and ${withWho} read this.`}
            </p>
          </div>
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
    </div>
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
          "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px] transition-colors hover:bg-muted",
          active && "bg-muted",
        )}
      >
        {children}
        {unread > 0 && (
          <span className="inline-flex min-w-4 shrink-0 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10.5px] leading-4 font-bold text-primary-foreground">
            <span aria-hidden="true">{unread}</span>
            <span className="sr-only"> unread</span>
          </span>
        )}
      </button>
    </li>
  );
}

// ---------------------------------------------------------------- thread

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
  const last = messages?.[messages.length - 1]?.seq ?? 0;

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [last, loading]);

  return (
    <div
      ref={box}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      role="log"
      aria-live="polite"
      className="scrollbar-thin h-[min(56dvh,520px)] min-h-64 overflow-y-auto overflow-x-hidden px-4 py-3"
    >
      {loading && !messages ? (
        <RowSkeleton rows={3} />
      ) : error && !messages ? (
        <p className="py-6 text-center text-destructive">{errorMessage(error)}</p>
      ) : !messages || messages.length === 0 ? (
        <EmptyState band={false} title={direct ? `Nothing between you and ${who} yet` : "The pass is quiet"} className="border-0">
          {direct ? "Write below. Only the two of you read it." : "Say something below. Every chef of the pot reads it; it is kept a week."}
        </EmptyState>
      ) : (
        <ul className="space-y-3.5">
          {messages.map((m) => (
            <li key={m.id} className="flex min-w-0 gap-3">
              <Avatar name={m.from ?? "the kitchen"} className="mt-0.5 size-7" />
              <div className="min-w-0 flex-1">
                <p className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-[13px]">
                  <span className={cn("max-w-full truncate font-semibold", m.from === me && "text-primary")} title={m.from ?? undefined}>
                    {m.from === me ? "you" : (m.from ?? "the kitchen")}
                  </span>
                  {m.from_kind === "agent" && <span className="label-caps">cook</span>}
                  {m.station_label && (
                    <span className="max-w-[14rem] truncate font-mono text-[11px] text-muted-foreground" title={`Session ${m.station}: ${m.station_label}`}>
                      · {m.station_label}
                    </span>
                  )}
                  {m.via && (
                    <span className="max-w-full truncate text-[12px] text-muted-foreground" title={`${m.via} posted it for ${m.from ?? "its owner"}`}>
                      via {m.via}
                    </span>
                  )}
                  <span className="font-mono text-[10.5px] text-muted-foreground" title={new Date(m.at).toLocaleString()}>
                    {fmtAgo(m.at, now)}
                  </span>
                  {m.task && (
                    <button
                      type="button"
                      onClick={() => openTask(m.task!)}
                      className="rounded-md border border-border bg-muted px-1.5 py-px hover:border-foreground/40 focus-visible:border-foreground/40"
                      title={`Open ${m.task}`}
                    >
                      <IdChip id={m.task} />
                    </button>
                  )}
                </p>
                <Markdown className="mt-0.5 min-w-0 text-[13.5px] break-words [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_table]:block [&_table]:overflow-x-auto">
                  {m.body}
                </Markdown>
                {m.woken.length > 0 && (
                  <p className="mt-0.5 truncate font-mono text-[10.5px] text-muted-foreground" title={m.woken.map(wokenName).join("\n")}>
                    woke {m.woken.length === 1 ? wokenName(m.woken[0]!) : `${m.woken.length} sessions: ${m.woken.map(wokenName).join(", ")}`}
                  </p>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- composer

/** Write on the pass (or to one chef). Enter sends, Shift+Enter is a new line; `@` suggests chefs. */
function Composer({ potKey, withWho, chefs }: { potKey: string; withWho: string; chefs: MemberRow[] }) {
  const invalidate = useInvalidateAll();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [caret, setCaret] = useState(0);
  const [pick, setPick] = useState(0);
  const [everyone, setEveryone] = useState(false);
  const ta = useRef<HTMLTextAreaElement>(null);
  const channel = withWho === POT;

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
        to: channel ? null : withWho,
        task: null,
        fromOwner: null,
        broadcast: channel && everyone ? true : null,
        station: null,
      });
      void invalidate();
      if (sent.woken.length > 0) toast(`Woke ${sent.woken.length === 1 ? wokenName(sent.woken[0]!) : `${sent.woken.length} sessions`}`);
    } catch (e) {
      setText(body);
      toast.error("Could not send it", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="relative border-t border-border px-3 py-2.5">
      {options.length > 0 && (
        <ul role="listbox" aria-label="Mention a chef" className="absolute right-3 bottom-full left-3 mb-1 rounded-lg border border-border bg-popover p-1 shadow-md">
          {options.map((c, i) => (
            <li key={c.name} role="option" aria-selected={i === pick}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  insert(c.name);
                }}
                className={cn("flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px]", i === pick && "bg-muted")}
              >
                <Avatar name={c.name} />
                <span className="min-w-0 flex-1 truncate">{c.name}</span>
                {c.kind === "agent" && <span className="label-caps shrink-0">cook</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-end gap-2">
        <Textarea
          ref={ta}
          rows={1}
          value={text}
          aria-label={channel ? "Write on the pass" : `Write to ${withWho}`}
          placeholder={channel ? "Write to the pass…" : `Write to ${withWho}…`}
          className="max-h-40 min-h-9 resize-none"
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
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <Button type="button" onClick={() => void send()} disabled={busy || !text.trim()} aria-label="Send">
          {busy ? <Loader2 className="animate-spin" /> : <Send />}
          <span className="max-sm:sr-only">Send</span>
        </Button>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
        {channel && (
          <label className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground" title="Wake every session of every cook in this pot (same as writing @all)">
            <input type="checkbox" checked={everyone} onChange={(e) => setEveryone(e.target.checked)} className="size-3.5 accent-[var(--primary)]" />
            Wake everyone
          </label>
        )}
        <p className="font-mono text-[10.5px] text-muted-foreground">
          ↵ to send · shift+↵ for a new line · markdown ok · {channel ? "@name wakes a cook, @all every session" : "a message to a cook wakes it"} · kept a week
        </p>
      </div>
    </div>
  );
}

/** Start a direct conversation with one of the pot's chefs. */
function NewDirect({ chefs, onPick }: { chefs: MemberRow[]; onPick: (name: string) => void }) {
  return (
    <Select value="" onValueChange={onPick}>
      <SelectTrigger size="sm" className="mt-2 w-full" aria-label="Write to a chef">
        <SelectValue placeholder="Write to a chef…" />
      </SelectTrigger>
      <SelectContent>
        {chefs.map((c) => (
          <SelectItem key={c.name} value={c.name}>
            {c.name}
            {c.kind === "agent" ? " · cook" : ""}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
