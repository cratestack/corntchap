import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";
import type { QuestionView } from "@corntchap/api";
import { Check, ChevronRight, Hourglass, Loader2, MessageCircleQuestion, Plus, Send, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall, useQuestions } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Avatar, EmptyState, IdChip, RowSkeleton, SectionLabel } from "./bits";
import { Markdown } from "./markdown";

/** "waiting for an answer to PAY-Q3" (a task's result while it is parked on a question) -> "PAY-Q3". */
export function parkedQuestionRef(t: { status: string; result?: string | null }): string | null {
  if (t.status !== "blocked" || !t.result) return null;
  const m = /^waiting for an answer to (\S+)/.exec(t.result);
  return m ? m[1]!.replace(/[.,;]$/, "") : null;
}

export const questionLink = (pot: string, id: string) => `/pots/${pot}/questions?question=${encodeURIComponent(id)}`;

/** "answered by alice (via desktop-wow)" */
function answeredBy(q: QuestionView): string {
  const who = q.answered_by ?? "someone";
  return q.relayed_by ? `${who}, via ${q.relayed_by}` : who;
}

// ---------------------------------------------------------------- answering

/**
 * Option buttons (pick one, then confirm, so a stray click never wakes a cook) plus a free-text box.
 * Enter sends, Shift+Enter adds a line.
 */
export function AnswerControls({ q, compact = false, idPrefix }: { q: QuestionView; compact?: boolean; idPrefix: string }) {
  const call = useProcCall();
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function send(answer: string) {
    const a = answer.trim();
    if (!a || busy) return;
    setBusy(true);
    try {
      await call("questionAnswer", { id: q.id, answer: a });
      toast.success(`Answered ${q.id}`, {
        description: q.wait && q.task ? `${q.task} goes back to the cook that asked.` : "The cook that asked is woken with your answer.",
      });
      setText("");
      setPicked(null);
    } catch (e) {
      toast.error("Could not send the answer", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  const inputId = `${idPrefix}-answer`;
  return (
    <div className="space-y-2.5">
      {q.options.length > 0 && (
        <div role="group" aria-label={`Suggested answers for ${q.id}`} className="flex flex-wrap gap-2">
          {q.options.map((o) => (
            <button
              key={o}
              type="button"
              aria-pressed={picked === o}
              disabled={busy}
              onClick={() => setPicked((p) => (p === o ? null : o))}
              className={cn(
                "max-w-full rounded-lg border px-3 py-1.5 text-left text-[13px] leading-snug transition-colors",
                picked === o
                  ? "border-primary bg-primary/10 font-medium text-foreground"
                  : "border-input bg-background hover:border-foreground/40",
              )}
            >
              {o}
            </button>
          ))}
        </div>
      )}
      {picked !== null && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-primary/40 bg-primary/5 px-3 py-2" role="status">
          <span className="min-w-0 flex-1 text-[13px]">
            Answer with <b className="font-semibold">“{picked}”</b>?
          </span>
          <Button size="sm" onClick={() => void send(picked)} disabled={busy}>
            {busy ? <Loader2 className="animate-spin" /> : <Check />} Send answer
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPicked(null)} disabled={busy}>
            Not that
          </Button>
        </div>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void send(text);
        }}
        className="space-y-1.5"
      >
        <Label htmlFor={inputId} className="sr-only">
          Your answer to {q.id}
        </Label>
        <div className="flex items-end gap-2">
          <Textarea
            id={inputId}
            value={text}
            rows={compact ? 1 : 2}
            onChange={(e) => {
              setText(e.target.value);
              if (e.target.value) setPicked(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void send(text);
              }
            }}
            placeholder={q.options.length > 0 ? "Or write your own answer…" : "Your answer…"}
            className="min-h-0 flex-1"
          />
          <Button type="submit" disabled={busy || !text.trim()} aria-label={`Send answer to ${q.id}`}>
            {busy ? <Loader2 className="animate-spin" /> : <Send />} <span className="max-sm:sr-only">Answer</span>
          </Button>
        </div>
        <p className="font-mono text-[10.5px] text-muted-foreground">↵ to send · shift+↵ for a new line · markdown ok</p>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------- cards

function Pill({ children, tone, className }: { children: React.ReactNode; tone: "corn" | "plain"; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-px text-[11.5px] font-medium",
        tone === "corn" ? "border-corn/60 bg-corn/15 text-corn-text" : "border-border bg-muted/60 text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

function TaskChip({ id }: { id: string }) {
  const open = useOpenTask();
  return (
    <button
      type="button"
      onClick={() => open(id)}
      title={`Open ${id}`}
      className="rounded-md border border-border bg-muted/50 px-1.5 py-px hover:border-foreground/40 focus-visible:border-foreground/40"
    >
      <IdChip id={id} className="text-[11.5px]" />
    </button>
  );
}

function Thread({ q }: { q: QuestionView }) {
  const now = useNow();
  if (q.notes.length === 0) return null;
  return (
    <ul className="space-y-2.5 border-l-2 border-border pl-3.5" aria-label={`Thread on ${q.id}`}>
      {q.notes.map((n, i) => (
        <li key={i} className="flex gap-2.5">
          <Avatar name={n.author ?? "?"} className="mt-0.5 size-5 text-[10px]" />
          <div className="min-w-0 flex-1">
            <p className="text-[12px] text-muted-foreground">
              <span className="font-semibold text-foreground">{n.author ?? "someone"}</span> · {fmtAgo(n.at, now)}
            </p>
            <Markdown className="text-[13.5px]">{n.body}</Markdown>
          </div>
        </li>
      ))}
    </ul>
  );
}

function NoteForm({ q }: { q: QuestionView }) {
  const call = useProcCall();
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    const n = body.trim();
    if (!n || busy) return;
    setBusy(true);
    try {
      await call("questionNote", { id: q.id, note: n });
      setBody("");
    } catch (err) {
      toast.error("Could not add the note", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <form onSubmit={submit} className="space-y-1">
      <Label htmlFor={`note-${q.id}`} className="sr-only">
        Reply in the thread on {q.id}
      </Label>
      <div className="flex gap-2">
        <Input
          id={`note-${q.id}`}
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Ask back or clarify…"
          className="h-8 text-[13px]"
        />
        <Button type="submit" size="sm" variant="outline" disabled={busy || !body.trim()}>
          {busy ? <Loader2 className="animate-spin" /> : <Send />} Reply
        </Button>
      </div>
      {q.asked_by && <p className="text-[11.5px] text-muted-foreground">{q.asked_by} will be woken to reply.</p>}
    </form>
  );
}

function DismissButton({ q }: { q: QuestionView }) {
  const call = useProcCall();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  async function dismiss(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await call("questionDismiss", { id: q.id, reason: reason.trim() || null });
      toast.success(`${q.id} dismissed`, { description: q.wait && q.task ? `${q.task} goes back to its cook.` : undefined });
      setOpen(false);
      setReason("");
    } catch (err) {
      toast.error("Could not dismiss it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className="text-muted-foreground">
          <X /> Dismiss
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 space-y-3">
        <form onSubmit={dismiss} className="space-y-3">
          <div>
            <p className="font-display text-[15px]">Dismiss {q.id}?</p>
            <p className="mt-0.5 text-[12px] text-muted-foreground">
              Closes it without an answer
              {q.wait && q.task ? `; ${q.task} goes back to its cook` : ""}. The cook is told why.
            </p>
          </div>
          <Label htmlFor={`dismiss-${q.id}`} className="sr-only">
            Reason (optional)
          </Label>
          <Input id={`dismiss-${q.id}`} autoFocus value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Reason (optional)" />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={() => setOpen(false)}>
              Keep it
            </Button>
            <Button type="submit" size="sm" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />} Dismiss
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}

function QuestionCard({ q, highlight }: { q: QuestionView; highlight: boolean }) {
  const now = useNow();
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (highlight) ref.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [highlight]);
  return (
    <li
      ref={ref}
      id={`q-${q.id}`}
      aria-label={`Question ${q.id}`}
      className={cn(
        "scroll-mt-24 space-y-3.5 rounded-[14px] border bg-card px-4 py-4",
        highlight ? "border-primary ring-2 ring-primary/30" : "border-border",
      )}
    >
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
        <Avatar name={q.asked_by ?? "?"} className="size-6 text-[11px]" />
        <span className="text-[13.5px] font-semibold">{q.asked_by ?? "someone"}</span>
        {q.to && <span className="text-[12.5px] text-muted-foreground">to {q.to}</span>}
        <span className="font-mono text-[11px] text-muted-foreground">
          {q.id} · {fmtAgo(q.created_at, now)}
        </span>
        <span className="ml-auto flex flex-wrap items-center gap-1.5">
          {q.task && <TaskChip id={q.task} />}
        </span>
      </div>

      {(q.wait || q.picked_by) && (
        <div className="flex flex-wrap gap-1.5">
          {q.wait && (
            <Pill tone="corn">
              <Hourglass className="size-3" aria-hidden="true" />
              {q.task ? `${q.task} is parked until you answer` : "waiting for an answer"}
            </Pill>
          )}
          {q.picked_by && (
            <Pill tone="plain">
              <span className="live-dot size-1.5 rounded-full bg-corn" aria-hidden="true" />
              being asked in {q.picked_by}’s chat
            </Pill>
          )}
        </div>
      )}

      <Markdown className="text-[14.5px]">{q.body}</Markdown>

      <Thread q={q} />

      <AnswerControls q={q} idPrefix={`q-${q.id}`} />

      <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2 border-t border-border pt-3">
        <div className="min-w-0 flex-1 basis-64">
          <NoteForm q={q} />
        </div>
        <DismissButton q={q} />
      </div>
    </li>
  );
}

function ClosedCard({ q, highlight }: { q: QuestionView; highlight: boolean }) {
  const now = useNow();
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (highlight) ref.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [highlight]);
  const dismissed = q.status === "dismissed";
  return (
    <li
      ref={ref}
      id={`q-${q.id}`}
      aria-label={`Question ${q.id}`}
      className={cn("scroll-mt-24 space-y-2 rounded-[14px] border bg-card/60 px-4 py-3.5", highlight ? "border-primary ring-2 ring-primary/30" : "border-border")}
    >
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="font-mono text-[11.5px] font-bold">{q.id}</span>
        <span className="text-[12.5px] text-muted-foreground">asked by {q.asked_by ?? "someone"}</span>
        {q.task && <TaskChip id={q.task} />}
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">{fmtAgo(q.answered_at ?? q.updated_at, now)}</span>
      </div>
      <Markdown className="text-[13.5px] text-muted-foreground">{q.body}</Markdown>
      <div
        className={cn(
          "rounded-lg border px-3 py-2",
          dismissed ? "border-border bg-muted/40" : "border-success/40 bg-success/10",
        )}
      >
        <SectionLabel className={dismissed ? "" : "!text-success"}>{dismissed ? "Dismissed" : "Answer"}</SectionLabel>
        {q.answer ? <Markdown className="mt-0.5 text-[13.5px]">{q.answer}</Markdown> : <p className="mt-0.5 text-[13px] text-muted-foreground">No reason given.</p>}
        <p className="mt-1 text-[12px] text-muted-foreground">
          {dismissed ? "dismissed" : "answered"} by {answeredBy(q)}
        </p>
      </div>
      {q.notes.length > 0 && <Thread q={q} />}
    </li>
  );
}

// ---------------------------------------------------------------- the view

export function QuestionsView({ potKey, onAsk }: { potKey: string; onAsk: () => void }) {
  const open = useQuestions(potKey, "open");
  const all = useQuestions(potKey, "all");
  const [params] = useSearchParams();
  const focus = params.get("question");
  const [showClosed, setShowClosed] = useState(false);

  const openList = useMemo(() => open.data ?? [], [open.data]);
  const closed = useMemo(() => (all.data ?? []).filter((q) => q.status !== "open"), [all.data]);
  const focusClosed = !!focus && closed.some((q) => q.id === focus);
  useEffect(() => {
    if (focusClosed) setShowClosed(true);
  }, [focusClosed]);

  if (open.isLoading) return <RowSkeleton rows={3} />;
  if (open.error) {
    return (
      <EmptyState band={false} title="Could not read the questions">
        {errorMessage(open.error)}
      </EmptyState>
    );
  }

  return (
    <div className="mx-auto max-w-3xl">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 className="text-[22px]">For the chef</h2>
        <p className="text-[13px] text-muted-foreground">
          {openList.length === 0
            ? "Nothing waiting on you."
            : `${plural(openList.length, "question")} waiting for a decision, oldest first.`}
        </p>
      </div>

      {openList.length === 0 ? (
        <EmptyState
          className="mt-4"
          title="No questions for the chef"
          action={
            <Button variant="outline" onClick={onAsk}>
              <Plus /> Ask a question
            </Button>
          }
        >
          Cooks ask here when they need a decision. Answer in the console, or in a chat with your own agent.
        </EmptyState>
      ) : (
        <ul className="mt-4 space-y-3" aria-label="Open questions">
          {openList.map((q) => (
            <QuestionCard key={q.id} q={q} highlight={focus === q.id} />
          ))}
        </ul>
      )}

      {closed.length > 0 && (
        <section className="mt-8" aria-labelledby="answered-questions">
          <h2 id="answered-questions" className="m-0">
            <button
              type="button"
              aria-expanded={showClosed}
              onClick={() => setShowClosed((o) => !o)}
              className="flex w-full items-center gap-2 rounded-md py-1 text-left"
            >
              <ChevronRight className={cn("size-4 text-muted-foreground transition-transform", showClosed && "rotate-90")} aria-hidden="true" />
              <span className="font-display text-[16px]">Answered</span>
              <span className="font-mono text-[11.5px] text-muted-foreground">{closed.length}</span>
            </button>
          </h2>
          {showClosed && (
            <ul className="mt-2 space-y-2.5">
              {closed.map((q) => (
                <ClosedCard key={q.id} q={q} highlight={focus === q.id} />
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- ask

export function AskQuestionDialog({
  potKey,
  open,
  onClose,
  task,
}: {
  potKey: string;
  open: boolean;
  onClose: () => void;
  task?: string;
}) {
  const call = useProcCall();
  const [question, setQuestion] = useState("");
  const [taskId, setTaskId] = useState("");
  const [options, setOptions] = useState("");
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setQuestion("");
      setTaskId(task ?? "");
      setOptions("");
      setTo("");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!question.trim()) return;
    setBusy(true);
    try {
      const q = await call("questionAsk", {
        question: question.trim(),
        project: potKey,
        task: taskId.trim().toUpperCase() || null,
        options: options
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean)
          .join("\n") || null,
        to: to.trim() || null,
      });
      toast.success(`${q.id} is in the pool`);
      onClose();
    } catch (err) {
      toast.error("Could not ask that", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Ask a question in {potKey}</DialogTitle>
          <DialogDescription>It goes into the pot’s pool for its people (or one person or cook) to answer.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="aq-body">Question</Label>
            <Textarea id="aq-body" autoFocus required rows={4} value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="Give enough context to answer it without reading the code." />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="aq-task">Task (optional)</Label>
              <Input id="aq-task" value={taskId} onChange={(e) => setTaskId(e.target.value)} placeholder={`${potKey}-42`} className="font-mono text-[12.5px]" />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="aq-to">Ask (optional)</Label>
              <Input id="aq-to" value={to} onChange={(e) => setTo(e.target.value)} placeholder="a person or cook" />
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="aq-opts">Options (optional, one per line)</Label>
            <Textarea id="aq-opts" rows={3} value={options} onChange={(e) => setOptions(e.target.value)} placeholder={"Yes\nNo"} className="font-mono text-[12.5px]" />
          </div>
          <DialogFooter className="gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !question.trim()}>
              {busy ? <Loader2 className="animate-spin" /> : <MessageCircleQuestion />} Ask it
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------- task sheet callout

/** Shown in the task sheet while the task is parked on a question: the question and an inline answer. */
export function ParkedQuestion({ id, pot }: { id: string; pot: string }) {
  const q = useProc("questionGet", { id });
  const data = q.data;
  if (!data || data.status !== "open") return null;
  return (
    <section aria-label={`Waiting on ${data.id}`} className="space-y-3 rounded-lg border border-corn/60 bg-corn/10 p-3.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SectionLabel className="!text-corn-text">
          <span className="inline-flex items-center gap-1.5">
            <Hourglass className="size-3" aria-hidden="true" /> Parked on {data.id}
          </span>
        </SectionLabel>
        <Link to={questionLink(pot, data.id)} className="text-[12.5px] text-primary underline-offset-2 hover:underline">
          Open in Questions
        </Link>
      </div>
      <p className="text-[12.5px] text-muted-foreground">
        {data.asked_by ?? "The cook"} asked and is waiting. Your answer puts the task back on its fire.
      </p>
      <Markdown className="text-[14px]">{data.body}</Markdown>
      <AnswerControls q={data} compact idPrefix={`parked-${data.id}`} />
    </section>
  );
}
