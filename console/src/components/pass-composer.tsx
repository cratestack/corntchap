import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { MemberView, StationView } from "@corntchap/api";
import { ArrowUp, Loader2, Megaphone, Plus, Terminal } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { client, errorMessage } from "@/lib/api";
import { useInvalidateAll } from "@/lib/data";
import { useSession } from "@/lib/session";
import { useTeamMembersSource, useStationsSource } from "@/lib/sources";
import type { ListSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { VirtualMenu } from "@/components/ui/virtual-menu";
import { MentionPopup, insertMention, parseMention } from "./mention-popup";
import { Avatar } from "./bits";
import { KindChip } from "./pass-bits";
import { POT, wokenName, type CookInfo } from "./pass-data";

export interface Seed {
  text: string;
  /** Changes with every request, so the same text can be seeded twice. */
  n: number;
}

interface Option {
  name: string;
  hint: string;
  kind: "user" | "agent" | "all";
}

/** The "+" button that opens the ways to send. */
const PLUS =
  "grid size-10 shrink-0 place-items-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground data-[state=open]:bg-muted pointer-fine:size-9";

const EVERYONE: Option = { name: "all", kind: "all", hint: "wake every session" };

const toOption = (m: MemberView): Option => ({ name: m.name, kind: m.kind === "agent" ? "agent" : "user", hint: m.display_name ?? "" });

/** The first row of the session menu: the cook's main session (no station named). */
type SessionRow = StationView | null;

/** Write on the pass (or to one chef). Enter sends on desktop, Shift+Enter is a new line; `@` suggests chefs. */
export function Composer({
  potKey,
  withWho,
  team,
  agent,
  cook,
  seed,
}: {
  potKey: string;
  withWho: string;
  /** The pot's team: who `@` suggests. */
  team: string | null;
  /** The conversation is with a cook (its sessions can be picked). */
  agent: boolean;
  cook: CookInfo | undefined;
  seed: Seed | null;
}) {
  const me = useSession();
  const invalidate = useInvalidateAll();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [caret, setCaret] = useState(0);
  const [everyone, setEveryone] = useState(false);
  const [session, setSession] = useState<StationView | null>(null);
  const sessionQuery = useStoreValue("");
  const ta = useRef<HTMLTextAreaElement>(null);
  const channel = withWho === POT;
  // a direct message to a cook can go to one of its sessions (stations) rather than its main one
  const toCook = !channel && agent;
  const sessions = useStationsSource({ project: potKey, cook: withWho, kind: "session", enabled: toCook, q: sessionQuery.value });
  // whether there is a choice does not follow the menu's search: it is read from the unsearched list
  const allSessions = useStationsSource({ project: potKey, cook: withWho, kind: "session", enabled: toCook });
  const hasOptions = channel || (toCook && (allSessions.items.length > 1 || allSessions.hasNextPage));
  const sessionRows = useMemo<ListSource<SessionRow>>(() => ({ ...sessions, items: [null, ...sessions.items] }), [sessions]);

  // a quote from "reply" lands in the box, with the cursor after it
  useEffect(() => {
    if (!seed) return;
    setText((t) => (t.trim() ? `${t.trimEnd()}\n\n${seed.text}` : seed.text));
    requestAnimationFrame(() => {
      const el = ta.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
      setCaret(el.value.length);
    });
  }, [seed]);

  // grow with the text, up to about six lines
  useLayoutEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  const mention = parseMention(text, caret);
  const members = useTeamMembersSource(team ?? "", { q: mention?.q ?? "", enabled: !!mention && !!team });
  const mentionSource = useMemo<ListSource<Option>>(() => {
    const rows = members.items.filter((m) => m.name !== me.name).map(toOption);
    return { ...members, items: channel && mention && EVERYONE.name.startsWith(mention.q) ? [EVERYONE, ...rows] : rows };
  }, [members, me.name, channel, mention]);

  function insert(name: string) {
    if (!mention) return;
    const next = insertMention(text, caret, mention, name);
    setText(next.text);
    setCaret(next.caret);
    requestAnimationFrame(() => {
      ta.current?.focus();
      ta.current?.setSelectionRange(next.caret, next.caret);
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
        to: channel ? null : (session?.id ?? withWho),
        task: null,
        fromOwner: null,
        broadcast: channel && everyone ? true : null,
        station: null,
      });
      void invalidate();
      setEveryone(false);
      if (sent.woken.length > 0)
        toast(`Woke ${sent.woken.length === 1 ? wokenName(sent.woken[0]!) : `${sent.woken.length} sessions`}`, {
          ...(sent.note ? { description: sent.note } : {}),
        });
    } catch (e) {
      setText(body);
      toast.error("Could not send it", { description: errorMessage(e) });
    } finally {
      setBusy(false);
      if (!window.matchMedia("(pointer: coarse)").matches) ta.current?.focus();
    }
  }

  const isTouch = () => window.matchMedia("(pointer: coarse)").matches;
  const ready = !!text.trim() && !busy;

  return (
    <div className="group/composer relative bg-card px-2.5 pt-1.5 pb-2.5 md:px-5 md:pt-2 md:pb-2">
      <div className="mx-auto max-w-3xl">
        {(everyone && channel) || (session && toCook) ? (
          <p className="mb-1.5 flex flex-wrap gap-x-3 px-2 text-[11.5px] text-muted-foreground">
            {everyone && channel && (
              <span className="inline-flex items-center gap-1 font-medium text-primary">
                <Megaphone className="size-3" aria-hidden="true" /> Broadcast: wakes every session
              </span>
            )}
            {session && toCook && (
              <span className="inline-flex items-center gap-1 font-mono">
                <Terminal className="size-3" aria-hidden="true" /> to session {session.label ?? session.directory ?? session.session}
              </span>
            )}
          </p>
        ) : null}
        <div className="flex items-end gap-1 rounded-[22px] border border-border bg-background py-1 pr-1 pl-1 transition-[border-color,box-shadow] focus-within:border-foreground/30 focus-within:ring-3 focus-within:ring-ring/15">
          {hasOptions && channel && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button type="button" aria-label="More ways to send" className={cn(PLUS, everyone && "bg-primary/12 text-primary")}>
                  <Plus className="size-5" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" side="top" className="min-w-64">
                <DropdownMenuLabel className="label-caps py-1">Send</DropdownMenuLabel>
                <DropdownMenuCheckboxItem checked={everyone} onCheckedChange={setEveryone} className="min-h-11 items-start gap-2 py-2">
                  <Megaphone className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" />
                  <span className="min-w-0">
                    Broadcast to everyone
                    <span className="block text-[11.5px] text-muted-foreground">Wakes every session of every cook, like @all</span>
                  </span>
                </DropdownMenuCheckboxItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          {hasOptions && !channel && (
            <VirtualMenu<SessionRow>
              label="To a session"
              mode="single"
              source={sessionRows}
              onQuery={sessionQuery.set}
              itemKey={(st) => st?.id ?? "main"}
              selected={(st) => (st?.id ?? "") === (session?.id ?? "")}
              onSelect={setSession}
              renderItem={(st) =>
                st ? <span className="block truncate">{(st.label ?? st.directory ?? st.session) + (st.main ? " (main)" : "")}</span> : <span>Its main session</span>
              }
              trigger={
                <button type="button" aria-label="More ways to send" className={cn(PLUS, session && "bg-primary/12 text-primary")}>
                  <Plus className="size-5" />
                </button>
              }
            />
          )}
          <MentionPopup<Option>
            open={mention !== null}
            query={mention?.q ?? ""}
            source={mentionSource}
            itemKey={(o) => o.name}
            label="Mention a chef"
            renderItem={(o) => (
              <span className="flex min-h-9 items-center gap-2.5 text-[14px]">
                {o.kind === "all" ? (
                  <span className="grid size-6 place-items-center rounded-md bg-primary/15 text-primary">
                    <Megaphone className="size-3.5" aria-hidden="true" />
                  </span>
                ) : (
                  <Avatar name={o.name} className="rounded-lg" />
                )}
                <span className="min-w-0 flex-1 truncate">
                  <span className="font-medium">@{o.name}</span>
                  {o.hint && <span className="ml-1.5 text-[12px] text-muted-foreground">{o.hint}</span>}
                </span>
                {o.kind === "agent" && <KindChip>cook</KindChip>}
              </span>
            )}
            onPick={(o) => insert(o.name)}
            onClose={() => setCaret(0)}
            anchor={(bind) => (
              <textarea
                {...bind}
                ref={ta}
                rows={1}
                value={text}
                aria-label={channel ? "Write on the pass" : `Write to ${withWho}`}
                placeholder={channel ? "Write to the pass…" : cook?.cloud ? `Brief ${withWho}…` : `Message ${withWho}…`}
                className={cn(
                  "scrollbar-thin max-h-40 min-h-10 min-w-0 flex-1 resize-none self-center bg-transparent py-2 text-base leading-snug outline-none placeholder:text-muted-foreground md:text-[14px] pointer-fine:min-h-9 pointer-fine:py-1.5",
                  !hasOptions && "pl-3",
                )}
                onChange={(e) => {
                  setText(e.target.value);
                  setCaret(e.target.selectionStart ?? e.target.value.length);
                }}
                onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
                onKeyDown={(e) => {
                  // the open mention list takes the arrows, Enter, Tab and Escape
                  bind.onKeyDown(e);
                  if (e.defaultPrevented) return;
                  // on a phone Enter is a new line (the send button sends); with a keyboard Enter sends
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && !isTouch()) {
                    e.preventDefault();
                    void send();
                  }
                }}
              />
            )}
          />
          <button
            type="button"
            onClick={() => void send()}
            disabled={!ready}
            aria-label="Send"
            className={cn(
              "grid size-10 shrink-0 place-items-center rounded-full transition-colors pointer-fine:size-9",
              ready ? "bg-primary text-primary-foreground hover:bg-primary/90 active:scale-95" : "bg-muted text-muted-foreground/70",
            )}
          >
            {busy ? <Loader2 className="size-[18px] animate-spin" /> : <ArrowUp className="size-5" strokeWidth={2.4} />}
          </button>
        </div>
        {/* the hint only shows while you type with a keyboard; the space is kept so nothing jumps */}
        <p
          aria-hidden="true"
          className="mt-1 hidden h-4 px-3 text-[11px] text-muted-foreground opacity-0 transition-opacity group-focus-within/composer:opacity-100 pointer-fine:block"
        >
          ↵ send · shift ↵ new line · @ mention{channel ? "" : cook ? " · wakes " + withWho : ""}
        </p>
      </div>
    </div>
  );
}
