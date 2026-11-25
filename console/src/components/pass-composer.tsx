import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, Loader2, Megaphone, Plus, Terminal } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { client, errorMessage } from "@/lib/api";
import { useInvalidateAll, useProc, type MemberRow } from "@/lib/data";
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

/** Write on the pass (or to one chef). Enter sends on desktop, Shift+Enter is a new line; `@` suggests chefs. */
export function Composer({
  potKey,
  withWho,
  chefs,
  cook,
  seed,
}: {
  potKey: string;
  withWho: string;
  chefs: MemberRow[];
  cook: CookInfo | undefined;
  seed: Seed | null;
}) {
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
  const stations = useProc("stationList", { project: potKey, cook: withWho, kind: "session" }, { enabled: toCook });
  const stationList = stations.data ?? [];
  const hasOptions = channel || (toCook && stationList.length > 1);

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

  const mention = useMemo(() => {
    const m = /(^|\s)@([\w.-]*)$/.exec(text.slice(0, caret));
    return m ? { start: caret - m[2]!.length - 1, q: m[2]!.toLowerCase() } : null;
  }, [text, caret]);
  const options = useMemo<Option[]>(() => {
    if (!mention) return [];
    const all: Option[] = chefs.map((c) => ({ name: c.name, kind: c.kind === "agent" ? "agent" : "user", hint: c.display_name ?? "" }));
    if (channel) all.push({ name: "all", kind: "all", hint: "wake every session" });
    const starts = all.filter((o) => o.name.toLowerCase().startsWith(mention.q));
    const has = all.filter((o) => !o.name.toLowerCase().startsWith(mention.q) && o.name.toLowerCase().includes(mention.q));
    return [...starts, ...has].slice(0, 6);
  }, [mention, chefs, channel]);
  const active = Math.min(pick, Math.max(options.length - 1, 0));

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
        {options.length > 0 && (
          <ul
            role="listbox"
            aria-label="Mention a chef"
            className="absolute right-2.5 bottom-full left-2.5 z-20 mb-1 max-h-72 overflow-y-auto rounded-xl border border-border bg-popover p-1 shadow-lg md:right-auto md:left-5 md:w-80"
          >
            {options.map((o, i) => (
              <li key={o.name} role="option" id={`mention-${o.name}`} aria-selected={i === active}>
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    insert(o.name);
                  }}
                  onMouseMove={() => setPick(i)}
                  className={cn("flex min-h-11 w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left text-[14px]", i === active && "bg-muted")}
                >
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
                </button>
              </li>
            ))}
          </ul>
        )}
        {(everyone && channel) || (session && toCook) ? (
          <p className="mb-1.5 flex flex-wrap gap-x-3 px-2 text-[11.5px] text-muted-foreground">
            {everyone && channel && (
              <span className="inline-flex items-center gap-1 font-medium text-primary">
                <Megaphone className="size-3" aria-hidden="true" /> Broadcast: wakes every session
              </span>
            )}
            {session && toCook && (
              <span className="inline-flex items-center gap-1 font-mono">
                <Terminal className="size-3" aria-hidden="true" /> to session {stationList.find((s) => s.id === session)?.label ?? session}
              </span>
            )}
          </p>
        ) : null}
        <div className="flex items-end gap-1 rounded-[22px] border border-border bg-background py-1 pr-1 pl-1 transition-[border-color,box-shadow] focus-within:border-foreground/30 focus-within:ring-3 focus-within:ring-ring/15">
          {hasOptions && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label="More ways to send"
                  className={cn(
                    "grid size-10 shrink-0 place-items-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground data-[state=open]:bg-muted pointer-fine:size-9",
                    (everyone || session) && "bg-primary/12 text-primary",
                  )}
                >
                  <Plus className="size-5" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" side="top" className="min-w-64">
                {channel ? (
                  <>
                    <DropdownMenuLabel className="label-caps py-1">Send</DropdownMenuLabel>
                    <DropdownMenuCheckboxItem checked={everyone} onCheckedChange={setEveryone} className="min-h-11 items-start gap-2 py-2">
                      <Megaphone className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" />
                      <span className="min-w-0">
                        Broadcast to everyone
                        <span className="block text-[11.5px] text-muted-foreground">Wakes every session of every cook, like @all</span>
                      </span>
                    </DropdownMenuCheckboxItem>
                  </>
                ) : (
                  <>
                    <DropdownMenuLabel className="label-caps flex items-center gap-1.5 py-1">
                      <Terminal className="size-3" aria-hidden="true" /> To a session…
                    </DropdownMenuLabel>
                    <DropdownMenuSeparator />
                    <DropdownMenuRadioGroup value={session} onValueChange={setSession}>
                      <DropdownMenuRadioItem value="" className="min-h-10">
                        Its main session
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
          <textarea
            ref={ta}
            rows={1}
            value={text}
            aria-label={channel ? "Write on the pass" : `Write to ${withWho}`}
            aria-activedescendant={options.length > 0 ? `mention-${options[active]!.name}` : undefined}
            placeholder={channel ? "Write to the pass…" : cook?.cloud ? `Brief ${withWho}…` : `Message ${withWho}…`}
            className={cn(
              "scrollbar-thin max-h-40 min-h-10 min-w-0 flex-1 resize-none self-center bg-transparent py-2 text-base leading-snug outline-none placeholder:text-muted-foreground md:text-[14px] pointer-fine:min-h-9 pointer-fine:py-1.5",
              !hasOptions && "pl-3",
            )}
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
                  setPick((active + (e.key === "ArrowDown" ? 1 : options.length - 1)) % options.length);
                  return;
                }
                if (e.key === "Enter" || e.key === "Tab") {
                  e.preventDefault();
                  insert(options[active]!.name);
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setCaret(0);
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
