import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";
import type { QuestionView } from "@corntchap/api";
import { Check, Hourglass, Loader2, MessageCircle, MessageCircleQuestion, Plus, Send, X } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall, useQuestions } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useIsPhone } from "@/lib/media";
import { projectOfTask, useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Avatar, EmptyState, IdChip, RowSkeleton, SectionLabel } from "./bits";
import { Markdown } from "./markdown";
import { Attachments } from "./files";

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
export function AnswerControls({
  q,
  compact = false,
  idPrefix,
  initialPick = null,
  onDone,
}: {
  q: QuestionView;
  compact?: boolean;
  idPrefix: string;
  initialPick?: string | null;
  onDone?: () => void;
}) {
  const call = useProcCall();
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<string | null>(initialPick);
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
      onDone?.();
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
                "min-h-11 max-w-full rounded-xl border px-3.5 py-2 text-left text-[14px] leading-snug transition-colors",
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
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && !window.matchMedia("(pointer: coarse)").matches) {
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
        <p className="hidden font-mono text-[10.5px] text-muted-foreground sm:block">↵ to send · shift+↵ for a new line · markdown ok</p>
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

function DismissForm({ q, onDone }: { q: QuestionView; onDone: () => void }) {
  const call = useProcCall();
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  async function dismiss(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await call("questionDismiss", { id: q.id, reason: reason.trim() || null });
      toast.success(`${q.id} dismissed`, { description: q.wait && q.task ? `${q.task} goes back to its cook.` : undefined });
      onDone();
    } catch (err) {
      toast.error("Could not dismiss it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <form onSubmit={dismiss} className="space-y-3">
      <p className="text-[13px] text-muted-foreground">
        Closes it without an answer
        {q.wait && q.task ? `; ${q.task} goes back to its cook` : ""}. The cook is told why.
      </p>
      <Label htmlFor={`dismiss-${q.id}`} className="sr-only">
        Reason (optional)
      </Label>
      <Textarea id={`dismiss-${q.id}`} rows={2} value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Reason (optional)" />
      <Button type="submit" variant="outline" className="w-full text-destructive hover:text-destructive" disabled={busy}>
        {busy ? <Loader2 className="animate-spin" /> : <X />} Dismiss {q.id}
      </Button>
    </form>
  );
}

type Mode = "answer" | "note" | "dismiss";
type SheetState = { id: string | null; mode: Mode; option?: string | undefined };

/** Answer, reply or dismiss one question: a bottom sheet on phones, a side sheet on wider screens. */
function QuestionSheet({ state, setMode, onClose }: { state: SheetState; setMode: (m: Mode) => void; onClose: () => void }) {
  const phone = useIsPhone();
  const { id, mode, option } = state;
  const res = useProc("questionGet", { id: id ?? "" }, { enabled: !!id });
  const q = res.data;
  const now = useNow();
  return (
    <Sheet open={!!id} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side={phone ? "bottom" : "right"} className="gap-0 px-0 pt-7 pb-3 max-sm:max-h-[90dvh] sm:max-w-md sm:pt-4" showCloseButton={false}>
        <SheetTitle className="sr-only">Question {id}</SheetTitle>
        <SheetDescription className="sr-only">Answer, reply in the thread, or dismiss the question</SheetDescription>
        {!q ? (
          <div className="space-y-3 p-5 pt-8" aria-busy="true">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-16 w-full" />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="flex items-center gap-2 px-4 pb-2">
              <Avatar name={q.asked_by ?? "?"} className="size-7 text-[12px]" />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[14px] font-semibold">
                  {q.asked_by ?? "someone"}
                  {q.to && <span className="font-normal text-muted-foreground"> to {q.to}</span>}
                </p>
                <p className="font-mono text-[11px] text-muted-foreground">
                  {q.id} · {fmtAgo(q.created_at, now)}
                </p>
              </div>
              <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Close">
                <X />
              </Button>
            </div>
            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain px-4 pb-3">
              <Markdown className="text-[14.5px]">{q.body}</Markdown>
              <Thread q={q} />
            </div>
            <div className="space-y-3 border-t border-border px-4 pt-3 pb-1">
              <div role="tablist" aria-label="Reply as" className="grid grid-cols-3 gap-1 rounded-xl bg-muted p-1">
                {(["answer", "note", "dismiss"] as const).map((m) => (
                  <button
                    key={m}
                    type="button"
                    role="tab"
                    aria-selected={mode === m}
                    onClick={() => setMode(m)}
                    className={cn(
                      "min-h-9 rounded-lg text-[13px] font-medium capitalize transition-colors",
                      mode === m ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
                    )}
                  >
                    {m === "note" ? "Reply" : m}
                  </button>
                ))}
              </div>
              {q.status !== "open" ? (
                <p className="text-[13px] text-muted-foreground">This question is {q.status === "dismissed" ? "dismissed" : "answered"}.</p>
              ) : mode === "answer" ? (
                <AnswerControls q={q} idPrefix={`sheet-${q.id}`} initialPick={option ?? null} onDone={onClose} />
              ) : mode === "note" ? (
                <NoteForm q={q} />
              ) : (
                <DismissForm q={q} onDone={onClose} />
              )}
            </div>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

function QuestionCard({
  q,
  highlight,
  onOpen,
}: {
  q: QuestionView;
  highlight: boolean;
  onOpen: (id: string, mode: Mode, option?: string) => void;
}) {
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
        "relative scroll-mt-24 space-y-3 overflow-hidden rounded-[14px] border bg-card py-3.5 pr-3.5 pl-4.5 before:absolute before:inset-y-0 before:left-0 before:w-1.5 before:bg-primary md:pr-4 md:pl-5",
        highlight ? "border-primary ring-2 ring-primary/30" : "border-border",
      )}
    >
      <div className="flex items-center gap-2.5">
        <Avatar name={q.asked_by ?? "?"} className="size-8 text-[13px]" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-[14px] leading-tight font-semibold">
            {q.asked_by ?? "someone"}
            {q.to && <span className="font-normal text-muted-foreground"> to {q.to}</span>}
          </p>
          <p className="font-mono text-[11px] text-muted-foreground">
            {q.id} · {fmtAgo(q.created_at, now)}
          </p>
        </div>
        {q.task && <TaskChip id={q.task} />}
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

      {/* an empty drop zone on every card is noise on a phone: there it shows only once files exist */}
      <div className={cn(q.files.length === 0 && "max-md:hidden")}>
        <Attachments compact target={{ pot: projectOfTask(q.id), question: q.id }} files={q.files} />
      </div>

      {q.options.length > 0 && (
        <div role="group" aria-label={`Suggested answers for ${q.id}`} className="flex flex-wrap gap-2">
          {q.options.map((o) => (
            <button
              key={o}
              type="button"
              onClick={() => onOpen(q.id, "answer", o)}
              className="min-h-11 max-w-full rounded-xl border border-input bg-background px-3.5 py-2 text-left text-[14px] leading-snug font-medium transition-colors hover:border-primary/60 hover:bg-primary/5 active:bg-primary/10"
            >
              {o}
            </button>
          ))}
        </div>
      )}

      {q.notes.length > 0 && (
        <button type="button" onClick={() => onOpen(q.id, "note")} className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground hover:text-foreground">
          <MessageCircle className="size-3.5" aria-hidden="true" /> {plural(q.notes.length, "reply", "replies")}
        </button>
      )}

      <div className="flex items-center gap-2 border-t border-border pt-3">
        <Button className="flex-1 sm:flex-none" onClick={() => onOpen(q.id, "answer")}>
          <Send /> Answer
        </Button>
        <Button variant="outline" onClick={() => onOpen(q.id, "note")}>
          <MessageCircle /> Reply
        </Button>
        <Button variant="ghost" className="text-muted-foreground" onClick={() => onOpen(q.id, "dismiss")}>
          <X /> <span className="max-sm:sr-only">Dismiss</span>
        </Button>
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
      className={cn("relative scroll-mt-24 space-y-2 overflow-hidden rounded-[14px] border bg-card/70 py-3.5 pr-3.5 pl-4.5 before:absolute before:inset-y-0 before:left-0 before:w-1.5", dismissed ? "before:bg-muted-foreground/40" : "before:bg-success", highlight ? "border-primary ring-2 ring-primary/30" : "border-border")}
    >
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="font-mono text-[11.5px] font-bold">{q.id}</span>
        <span className="text-[12.5px] text-muted-foreground">asked by {q.asked_by ?? "someone"}</span>
        {q.task && <TaskChip id={q.task} />}
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">{fmtAgo(q.answered_at ?? q.updated_at, now)}</span>
      </div>
      <Markdown className="text-[13.5px] text-muted-foreground">{q.body}</Markdown>
      {q.files.length > 0 && <Attachments compact target={{ pot: projectOfTask(q.id), question: q.id }} files={q.files} />}
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

type Filter = "open" | "answered" | "dismissed";

export function QuestionsView({ potKey, onAsk }: { potKey: string; onAsk: () => void }) {
  const open = useQuestions(potKey, "open");
  const all = useQuestions(potKey, "all");
  const [params] = useSearchParams();
  const focus = params.get("question");
  const [filter, setFilter] = useState<Filter>("open");
  const [sheet, setSheet] = useState<SheetState>({ id: null, mode: "answer" });

  const openList = useMemo(() => open.data ?? [], [open.data]);
  const answered = useMemo(() => (all.data ?? []).filter((q) => q.status === "answered"), [all.data]);
  const dismissed = useMemo(() => (all.data ?? []).filter((q) => q.status === "dismissed"), [all.data]);
  const focusFilter: Filter | null = !focus
    ? null
    : answered.some((q) => q.id === focus)
      ? "answered"
      : dismissed.some((q) => q.id === focus)
        ? "dismissed"
        : null;
  useEffect(() => {
    if (focusFilter) setFilter(focusFilter);
  }, [focusFilter]);

  if (open.isLoading) return <RowSkeleton rows={3} />;
  if (open.error) {
    return (
      <EmptyState band={false} title="Could not read the questions">
        {errorMessage(open.error)}
      </EmptyState>
    );
  }

  const tabs: { id: Filter; label: string; n: number }[] = [
    { id: "open", label: "Open", n: openList.length },
    { id: "answered", label: "Answered", n: answered.length },
    { id: "dismissed", label: "Dismissed", n: dismissed.length },
  ];
  const shown = filter === "answered" ? answered : dismissed;

  return (
    <div className="mx-auto max-w-3xl">
      <div className="flex items-center gap-2">
        <div role="tablist" aria-label="Question status" className="grid flex-1 grid-cols-3 gap-1 rounded-xl bg-muted p-1 sm:flex-none sm:grid-cols-[repeat(3,8rem)]">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={filter === t.id}
              onClick={() => setFilter(t.id)}
              className={cn(
                "flex min-h-10 items-center justify-center gap-1.5 rounded-lg text-[13px] font-medium transition-colors",
                filter === t.id ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              {t.label}
              <span
                className={cn(
                  "font-mono text-[11px]",
                  t.id === "open" && t.n > 0 ? "rounded-full bg-primary px-1.5 leading-4 text-primary-foreground" : "text-muted-foreground",
                )}
              >
                {t.n}
              </span>
            </button>
          ))}
        </div>
      </div>

      {filter === "open" ? (
        openList.length === 0 ? (
          <EmptyState
            className="mt-4"
            title="Nothing waiting on you"
            action={
              <Button variant="outline" onClick={onAsk}>
                <Plus /> Ask a question
              </Button>
            }
          >
            Cooks ask here when they need a decision. Answer in the console, or in a chat with your own agent.
          </EmptyState>
        ) : (
          <>
            <p className="label-caps mt-4 mb-2 px-0.5">{plural(openList.length, "question")} waiting · oldest first</p>
            <ul className="space-y-3" aria-label="Open questions">
              {openList.map((q) => (
                <QuestionCard key={q.id} q={q} highlight={focus === q.id} onOpen={(id, mode, option) => setSheet({ id, mode, option })} />
              ))}
            </ul>
          </>
        )
      ) : shown.length === 0 ? (
        <EmptyState band={false} className="mt-4" title={`No ${filter} questions`}>
          {filter === "answered" ? "Questions you answer show up here." : "Questions you dismiss show up here, with the reason you gave."}
        </EmptyState>
      ) : (
        <ul className="mt-4 space-y-2.5" aria-label={`${filter} questions`}>
          {shown.map((q) => (
            <ClosedCard key={q.id} q={q} highlight={focus === q.id} />
          ))}
        </ul>
      )}

      <QuestionSheet
        key={sheet.id ? `${sheet.id}/${sheet.option ?? ""}` : "none"}
        state={sheet}
        setMode={(mode) => setSheet((st) => ({ ...st, mode }))}
        onClose={() => setSheet((st) => ({ ...st, id: null }))}
      />
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
