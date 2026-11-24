import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { QuestionChoice, QuestionView } from "@corntchap/api";
import { Check, CornerDownLeft, Loader2, Send } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

export type Kind = "single" | "multiple" | "free" | "cards";

export const kindOf = (q: Pick<QuestionView, "kind">): Kind => (["single", "multiple", "free", "cards"].includes(q.kind) ? (q.kind as Kind) : "single");

/** "Pick one", "Pick 1 to 3", "Quick pick", "Write an answer": how a question wants to be answered. */
export function kindHint(q: Pick<QuestionView, "kind" | "min" | "max" | "allow_other">): string {
  const other = q.allow_other ? " or write your own" : "";
  switch (kindOf(q)) {
    case "free":
      return "Write an answer";
    case "cards":
      return `Quick pick${other}`;
    case "multiple": {
      const { min, max } = q;
      const range = min != null && max != null ? (min === max ? `Pick ${min}` : `Pick ${min} to ${max}`) : min != null ? `Pick at least ${min}` : max != null ? `Pick up to ${max}` : "Pick any";
      return `${range}${other}`;
    }
    default:
      return `Pick one${other}`;
  }
}

const OTHER = "__other__";

/** Whether the picks and text make a valid answer, and what to tell the person when they do not. */
function check(q: QuestionView, sel: string[], text: string): { ok: boolean; hint: string | null } {
  const kind = kindOf(q);
  const t = text.trim();
  if (kind === "free") return { ok: !!t, hint: t ? null : "Write your answer" };
  const other = sel.includes(OTHER);
  const n = sel.length;
  if (other && !t) return { ok: false, hint: "Write your own answer, or pick another option" };
  if (kind === "multiple") {
    if (n === 0) return { ok: false, hint: "Pick at least one" };
    if (q.min != null && n < q.min) return { ok: false, hint: `Pick at least ${q.min}` };
    if (q.max != null && n > q.max) return { ok: false, hint: `Pick at most ${q.max}` };
    return { ok: true, hint: null };
  }
  return n === 1 ? { ok: true, hint: null } : { ok: false, hint: "Pick one" };
}

/** A textarea that grows with its text. */
export function AutoTextarea({ value, onChange, className, minRows = 2, ...rest }: Omit<React.ComponentProps<"textarea">, "value" | "onChange"> & { value: string; onChange: (v: string) => void; minRows?: number }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + 2, 280)}px`;
  }, [value]);
  return (
    <textarea
      ref={ref}
      rows={minRows}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className={cn(
        "block w-full resize-none rounded-[10px] border border-input bg-background px-3 py-2.5 text-[14.5px] leading-relaxed outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/40",
        className,
      )}
      {...rest}
    />
  );
}

function Indicator({ type, cards, className }: { type: "radio" | "checkbox"; cards?: boolean; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid shrink-0 place-items-center border border-input bg-background text-primary-foreground transition-colors peer-checked:border-primary peer-checked:bg-primary",
        type === "radio" ? "size-5 rounded-full" : "size-5 rounded-[6px]",
        cards && "absolute top-2 right-2 size-[18px]",
        className,
        // the radio dot / the check, shown only when checked
        "[&>svg]:opacity-0 peer-checked:[&>svg]:opacity-100",
      )}
    >
      {type === "radio" ? <span className="size-2 rounded-full bg-primary-foreground" /> : <Check className="size-3.5" strokeWidth={3} />}
    </span>
  );
}

function Option({
  c,
  index,
  type,
  name,
  checked,
  disabled,
  onChange,
  card,
}: {
  c: Pick<QuestionChoice, "id" | "label"> & { description?: string | null | undefined };
  index: number;
  type: "radio" | "checkbox";
  name: string;
  checked: boolean;
  disabled?: boolean;
  onChange: () => void;
  card?: boolean;
}) {
  return (
    <label
      className={cn(
        "relative flex cursor-pointer rounded-xl border bg-card transition-colors select-none",
        "has-[:focus-visible]:ring-[3px] has-[:focus-visible]:ring-ring/50",
        checked ? "border-primary bg-primary/10 shadow-[inset_0_0_0_1px_var(--color-primary)]" : "border-input hover:border-foreground/40 active:bg-muted/60",
        disabled && !checked && "cursor-not-allowed opacity-50 hover:border-input",
        card ? "min-h-[4.5rem] flex-col items-center justify-center gap-0.5 px-8 py-3 text-center" : "min-h-12 items-start gap-3 px-3.5 py-3",
      )}
    >
      <input type={type} name={name} value={c.id} checked={checked} disabled={disabled} onChange={onChange} className="peer sr-only" />
      {card ? (
        <>
          <kbd className="absolute top-2 left-2.5 font-mono text-[10.5px] text-muted-foreground max-md:hidden pointer-coarse:hidden" aria-hidden="true">
            {index + 1}
          </kbd>
          <Indicator type="radio" cards />
          <span className="text-[15px] leading-snug font-semibold">{c.label}</span>
          {c.description && <span className="text-[12.5px] leading-snug text-muted-foreground">{c.description}</span>}
        </>
      ) : (
        <>
          <Indicator type={type} className="mt-px" />
          <span className="min-w-0 flex-1">
            <span className="block text-[14.5px] leading-snug font-medium">{c.label}</span>
            {c.description && <span className="mt-0.5 block text-[13px] leading-snug text-muted-foreground">{c.description}</span>}
          </span>
          <kbd className="mt-0.5 font-mono text-[10.5px] text-muted-foreground max-md:hidden pointer-coarse:hidden" aria-hidden="true">
            {index < 9 ? index + 1 : ""}
          </kbd>
        </>
      )}
    </label>
  );
}

/**
 * Answer one question, as its kind asks: radio cards, a radio list, checkboxes or free text, plus an
 * "Other" field when the question allows it. Pick, then send: a stray tap never wakes a cook.
 * Radio groups are native, so arrow keys move through them; 1-9 pick an option on desktop.
 */
export function AnswerControls({
  q,
  compact = false,
  idPrefix,
  initialPick = null,
  onDone,
  autoFocus = false,
  sticky = false,
}: {
  q: QuestionView;
  compact?: boolean;
  idPrefix: string;
  initialPick?: string | null;
  onDone?: () => void;
  autoFocus?: boolean;
  /** Keep the send bar at the bottom of a scrolling sheet. */
  sticky?: boolean;
}) {
  const call = useProcCall();
  const kind = kindOf(q);
  const [sel, setSel] = useState<string[]>(initialPick && q.choices.some((c) => c.id === initialPick) ? [initialPick] : []);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const name = `${idPrefix}-choice`;
  const otherOn = sel.includes(OTHER);
  const state = check(q, sel, text);
  const atMax = kind === "multiple" && q.max != null && sel.length >= q.max;

  useEffect(() => {
    if (!autoFocus) return;
    const first = rootRef.current?.querySelector<HTMLElement>("input:checked, input, textarea");
    first?.focus({ preventScroll: true });
  }, [autoFocus]);

  function pick(id: string) {
    if (kind === "multiple") setSel((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
    else setSel([id]);
  }

  async function send() {
    if (!state.ok || busy) return;
    setBusy(true);
    try {
      const chosen = sel.filter((x) => x !== OTHER);
      await call("questionAnswer", {
        id: q.id,
        choices: kind === "free" || chosen.length === 0 ? null : JSON.stringify(chosen),
        text: kind === "free" || otherOn ? text.trim() : null,
      });
      toast.success(`Answered ${q.id}`, {
        description: q.wait && q.task ? `${q.task} goes back to the cook that asked.` : "The cook that asked is woken with your answer.",
      });
      setText("");
      setSel([]);
      onDone?.();
    } catch (e) {
      toast.error("Could not send the answer", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.nativeEvent.isComposing || e.metaKey || e.ctrlKey || e.altKey) {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void send();
      }
      return;
    }
    const inText = (e.target as HTMLElement).tagName === "TEXTAREA";
    if (inText) {
      const coarse = window.matchMedia("(pointer: coarse)").matches;
      if (e.key === "Enter" && !e.shiftKey && !coarse) {
        e.preventDefault();
        void send();
      }
      return;
    }
    if ((kind === "cards" || kind === "single") && /^[1-9]$/.test(e.key)) {
      const c = q.choices[Number(e.key) - 1];
      if (c) {
        e.preventDefault();
        setSel([c.id]);
        rootRef.current?.querySelector<HTMLInputElement>(`input[value="${CSS.escape(c.id)}"]`)?.focus();
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      void send();
    }
  }

  const type = kind === "multiple" ? "checkbox" : "radio";
  const showOther = kind !== "free" && q.allow_other;
  const cards = kind === "cards";

  return (
    <div ref={rootRef} onKeyDown={onKeyDown} className="space-y-3">
      {kind === "free" ? (
        <div className="space-y-1.5">
          <Label htmlFor={`${idPrefix}-text`} className="sr-only">
            Your answer to {q.id}
          </Label>
          <AutoTextarea id={`${idPrefix}-text`} value={text} onChange={setText} minRows={compact ? 2 : 3} placeholder="Your answer…" />
        </div>
      ) : (
        <>
          <div
            role={type === "radio" ? "radiogroup" : "group"}
            aria-label={`${kindHint(q)} for ${q.id}`}
            className={cn(cards ? "grid grid-cols-2 gap-2.5 sm:grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))]" : "space-y-2")}
          >
            {q.choices.map((c, i) => (
              <Option
                key={c.id}
                c={cards ? { id: c.id, label: c.label, description: c.description } : c}
                index={i}
                type={type}
                name={name}
                card={cards}
                checked={sel.includes(c.id)}
                disabled={busy || (type === "checkbox" && atMax && !sel.includes(c.id))}
                onChange={() => pick(c.id)}
              />
            ))}
            {showOther && (
              <Option
                c={{ id: OTHER, label: "Other…", description: cards ? null : "Write your own answer" }}
                index={q.choices.length}
                type={type}
                name={name}
                card={cards}
                checked={otherOn}
                disabled={busy || (type === "checkbox" && atMax && !otherOn)}
                onChange={() => pick(OTHER)}
              />
            )}
          </div>
          {otherOn && (
            <div className="space-y-1.5">
              <Label htmlFor={`${idPrefix}-other`} className="sr-only">
                Your own answer to {q.id}
              </Label>
              <AutoTextarea id={`${idPrefix}-other`} autoFocus value={text} onChange={setText} minRows={2} placeholder="Your own answer…" />
            </div>
          )}
        </>
      )}

      <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-2 pt-1", sticky && "sticky bottom-0 z-10 -mx-1 bg-card px-1 pb-0.5")}>
        <p className="min-w-0 flex-1 text-[12.5px] text-muted-foreground" role="status">
          {state.hint ?? (kind === "multiple" ? `${sel.length} picked${q.max != null ? ` of ${q.max}` : ""}` : "Ready to send")}
          {kind === "multiple" && state.hint && sel.length > 0 && <span className="font-mono"> · {sel.length}</span>}
        </p>
        <Button className="max-sm:w-full" onClick={() => void send()} disabled={busy || !state.ok}>
          {busy ? <Loader2 className="animate-spin" /> : <Send />} Send answer
          <CornerDownLeft className="size-3.5 opacity-60 max-md:hidden" aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}

/** The answer of a closed question: chosen options as chips, the typed text below. */
export function AnswerSummary({ q }: { q: QuestionView }) {
  const chosen = q.answer_choices.map((id) => q.choices.find((c) => c.id === id)?.label ?? id);
  const text = q.answer_text?.trim();
  if (chosen.length === 0 && !text) {
    return q.answer ? <p className="text-[14px]">{q.answer}</p> : null;
  }
  return (
    <div className="space-y-1.5">
      {chosen.length > 0 && (
        <ul className="flex flex-wrap gap-1.5" aria-label="Chosen">
          {chosen.map((l) => (
            <li key={l} className="inline-flex items-center gap-1 rounded-full border border-success/50 bg-success/15 px-2.5 py-1 text-[13px] leading-none font-medium text-foreground">
              <Check className="size-3.5 text-success" aria-hidden="true" />
              {l}
            </li>
          ))}
        </ul>
      )}
      {text && (
        <p className={cn("text-[14px] leading-relaxed whitespace-pre-wrap", chosen.length > 0 && "border-l-2 border-success/50 pl-2.5")}>
          {chosen.length > 0 && <span className="mr-1 text-[12px] font-semibold text-muted-foreground">Own answer:</span>}
          {text}
        </p>
      )}
    </div>
  );
}
