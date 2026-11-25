import { useEffect, useRef, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";
import type { QuestionView } from "@corntchap/api";
import {
  ArrowDown,
  ArrowUp,
  CheckSquare,
  ChevronDown,
  Hourglass,
  LayoutGrid,
  ListChecks,
  Loader2,
  MessageCircle,
  MessageCircleQuestion,
  Pencil,
  Plus,
  Send,
  CircleDot,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall, useReviews } from "@/lib/data";
import { useProcInfinite } from "@/lib/paging";
import { errorMessage } from "@/lib/api";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useIsPhone } from "@/lib/media";
import { projectOfTask, useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { Skeleton } from "@/components/ui/skeleton";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Avatar, EmptyState, IdChip, RowSkeleton, SectionLabel } from "./bits";
import { Markdown } from "./markdown";
import { Attachments } from "./files";
import { AnswerControls, AnswerSummary, kindHint, kindOf } from "./question-answer";
import { ReviewsList } from "./reviews";

export { AnswerControls } from "./question-answer";

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

// ---------------------------------------------------------------- bits

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

const KIND_ICON = { cards: LayoutGrid, single: CircleDot, multiple: CheckSquare, free: Pencil } as const;

/** What kind of answer a question wants, as a small chip. */
function KindChip({ q }: { q: QuestionView }) {
  const Icon = KIND_ICON[kindOf(q)];
  return (
    <Pill tone="plain">
      <Icon className="size-3" aria-hidden="true" />
      {kindHint(q)}
    </Pill>
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

/** The description as markdown, clamped to a few lines with a toggle when it is long. */
function Description({ q, className, startOpen = false }: { q: QuestionView; className?: string; startOpen?: boolean }) {
  const [open, setOpen] = useState(startOpen);
  const d = q.description.trim();
  if (!d) return null;
  const long = d.length > 220 || d.split("\n").length > 4;
  return (
    <div className={className}>
      <div className={cn("relative", long && !open && "max-h-[4.9rem] overflow-hidden")}>
        <Markdown className="text-[14px] text-muted-foreground [&_p]:leading-relaxed">{d}</Markdown>
        {long && !open && <div className="pointer-events-none absolute inset-x-0 bottom-0 h-7 bg-gradient-to-t from-card to-transparent" aria-hidden="true" />}
      </div>
      {long && (
        <button type="button" onClick={() => setOpen((o) => !o)} aria-expanded={open} className="mt-1 inline-flex items-center gap-1 text-[12.5px] font-medium text-primary hover:underline">
          {open ? "Show less" : "Show more"} <ChevronDown className={cn("size-3.5 transition-transform", open && "rotate-180")} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

function Asker({ q, now }: { q: QuestionView; now: number }) {
  return (
    <div className="flex items-center gap-2.5">
      <Avatar name={q.asked_by ?? "?"} className="size-7 text-[12px]" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-[13.5px] leading-tight font-semibold">
          {q.asked_by ?? "someone"}
          {q.to && <span className="font-normal text-muted-foreground"> to {q.to}</span>}
        </p>
        <p className="font-mono text-[11px] text-muted-foreground">
          {q.id} · {fmtAgo(q.created_at, now)}
        </p>
      </div>
    </div>
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
      <SheetContent side={phone ? "bottom" : "right"} className="gap-0 px-0 pt-7 pb-3 max-sm:max-h-[92dvh] sm:max-w-md sm:pt-4" showCloseButton={false}>
        <SheetTitle className="sr-only">{q ? q.title : `Question ${id}`}</SheetTitle>
        <SheetDescription className="sr-only">Answer, reply in the thread, or dismiss the question</SheetDescription>
        {!q ? (
          <div className="space-y-3 p-5 pt-8" aria-busy="true">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-16 w-full" />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="flex items-start gap-2 px-4 pb-2">
              <div className="min-w-0 flex-1">
                <Asker q={q} now={now} />
              </div>
              <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Close">
                <X />
              </Button>
            </div>
            <div className="min-h-0 flex-1 space-y-3.5 overflow-y-auto overscroll-contain px-4 pb-3">
              <div className="space-y-2">
                <h2 className="font-display text-[19px] leading-snug font-extrabold text-balance">{q.title}</h2>
                <div className="flex flex-wrap gap-1.5">
                  <KindChip q={q} />
                  {q.task && <TaskChip id={q.task} />}
                </div>
              </div>
              <Description q={q} startOpen />
              <Thread q={q} />
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
                <div className="space-y-2">
                  <p className="text-[13px] text-muted-foreground">This question is {q.status === "dismissed" ? "dismissed" : "answered"}.</p>
                  <AnswerSummary q={q} />
                </div>
              ) : mode === "answer" ? (
                <AnswerControls q={q} idPrefix={`sheet-${q.id}`} initialPick={option ?? null} onDone={onClose} autoFocus={!phone} sticky />
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
  const phone = useIsPhone();
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (highlight) ref.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [highlight]);
  const quick = (kindOf(q) === "cards" || kindOf(q) === "single") && q.choices.length <= 4;
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
        <div className="min-w-0 flex-1">
          <Asker q={q} now={now} />
        </div>
        {q.task && <TaskChip id={q.task} />}
      </div>

      <div className="space-y-1.5">
        <h3 className="font-display text-[17px] leading-snug font-extrabold text-balance">{q.title}</h3>
        <div className="flex flex-wrap gap-1.5">
          <KindChip q={q} />
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
      </div>

      <Description q={q} />

      {/* an empty drop zone on every card is noise on a phone: there it shows only once files exist */}
      <div className={cn(q.files.length === 0 && "max-md:hidden")}>
        <Attachments compact target={{ pot: projectOfTask(q.id), question: q.id }} files={q.files} />
      </div>

      {phone && quick && (
        <div role="group" aria-label={`Quick answers for ${q.id}`} className="grid grid-cols-2 gap-2">
          {q.choices.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => onOpen(q.id, "answer", c.id)}
              className="min-h-12 rounded-xl border border-input bg-background px-3 py-2 text-center text-[14.5px] leading-snug font-semibold transition-colors active:bg-primary/10"
            >
              {c.label}
            </button>
          ))}
        </div>
      )}

      {!phone && (
        <div className="rounded-xl border border-border bg-background/60 p-3">
          <AnswerControls q={q} idPrefix={`card-${q.id}`} compact />
        </div>
      )}

      {q.notes.length > 0 && (
        <button type="button" onClick={() => onOpen(q.id, "note")} className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground hover:text-foreground">
          <MessageCircle className="size-3.5" aria-hidden="true" /> {plural(q.notes.length, "reply", "replies")}
        </button>
      )}

      <div className="flex items-center gap-2 border-t border-border pt-3">
        {phone && (
          <Button className="flex-1" onClick={() => onOpen(q.id, "answer")}>
            <Send /> Answer
          </Button>
        )}
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
      className={cn("relative scroll-mt-24 space-y-2.5 overflow-hidden rounded-[14px] border bg-card/70 py-3.5 pr-3.5 pl-4.5 before:absolute before:inset-y-0 before:left-0 before:w-1.5", dismissed ? "before:bg-muted-foreground/40" : "before:bg-success", highlight ? "border-primary ring-2 ring-primary/30" : "border-border")}
    >
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="font-mono text-[11.5px] font-bold">{q.id}</span>
        <span className="text-[12.5px] text-muted-foreground">asked by {q.asked_by ?? "someone"}</span>
        {q.task && <TaskChip id={q.task} />}
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">{fmtAgo(q.answered_at ?? q.updated_at, now)}</span>
      </div>
      <h3 className="font-display text-[16px] leading-snug font-extrabold text-balance">{q.title}</h3>
      <Description q={q} />
      {q.files.length > 0 && <Attachments compact target={{ pot: projectOfTask(q.id), question: q.id }} files={q.files} />}
      <div className={cn("space-y-1.5 rounded-lg border px-3 py-2.5", dismissed ? "border-border bg-muted/40" : "border-success/40 bg-success/10")}>
        <SectionLabel className={dismissed ? "" : "!text-success"}>{dismissed ? "Dismissed" : "Answer"}</SectionLabel>
        {dismissed ? (
          q.answer ? (
            <Markdown className="text-[13.5px]">{q.answer}</Markdown>
          ) : (
            <p className="text-[13px] text-muted-foreground">No reason given.</p>
          )
        ) : (
          <AnswerSummary q={q} />
        )}
        <p className="text-[12px] text-muted-foreground">
          {dismissed ? "dismissed" : "answered"} by {answeredBy(q)} · {fmtAgo(q.answered_at ?? q.updated_at, now)}
        </p>
      </div>
      {q.notes.length > 0 && <Thread q={q} />}
    </li>
  );
}

// ---------------------------------------------------------------- the view

type Filter = "open" | "answered" | "dismissed" | "reviews";

/** Questions per page of a status. */
const QUESTION_PAGE = 25;

export function QuestionsView({ potKey, onAsk }: { potKey: string; onAsk: () => void }) {
  const [params] = useSearchParams();
  const focus = params.get("question");
  const [filter, setFilter] = useState<Filter>("open");
  const [sheet, setSheet] = useState<SheetState>({ id: null, mode: "answer" });
  const useStatusPage = (status: "open" | "answered" | "dismissed", enabled: boolean) =>
    useProcInfinite("questionPage", { project: potKey, status }, { getCursor: (p) => p.nextCursor, pageSize: QUESTION_PAGE, enabled });
  const open = useStatusPage("open", true);
  // the closed lists load when their tab is open, or to find where a linked question is
  const answered = useStatusPage("answered", filter === "answered" || !!focus);
  const dismissed = useStatusPage("dismissed", filter === "dismissed" || !!focus);
  const waitingReviews = useReviews(potKey, "mine");
  const allReviews = useReviews(potKey, "all", filter === "reviews");

  const openList = open.items;
  const focusFilter: Filter | null = !focus
    ? null
    : answered.items.some((q) => q.id === focus)
      ? "answered"
      : dismissed.items.some((q) => q.id === focus)
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
    { id: "answered", label: "Answered", n: answered.items.length },
    { id: "dismissed", label: "Dismissed", n: dismissed.items.length },
    { id: "reviews", label: "Reviews", n: allReviews.data.length },
  ];
  const waiting = waitingReviews.data;
  const closed = filter === "answered" ? answered : dismissed;
  const shown = closed.items;

  return (
    <div className="mx-auto max-w-3xl">
      <div className="flex items-center gap-2">
        <div role="tablist" aria-label="Question status" className="grid flex-1 grid-cols-4 gap-1 rounded-xl bg-muted p-1 sm:flex-none sm:grid-cols-[repeat(4,7.5rem)]">
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
                  (t.id === "open" && t.n > 0) || (t.id === "reviews" && waiting.length > 0) ? "rounded-full bg-primary px-1.5 leading-4 text-primary-foreground" : "text-muted-foreground",
                )}
              >
                {t.id === "reviews" && waiting.length > 0 ? waiting.length : t.n}
              </span>
            </button>
          ))}
        </div>
      </div>

      {filter === "reviews" ? (
        <>
          <ReviewsList reviews={allReviews.data} loading={allReviews.isLoading} />
          <MoreButton more={allReviews} what="reviews" />
        </>
      ) : filter === "open" ? (
        openList.length === 0 && waiting.length === 0 ? (
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
            {waiting.length > 0 && (
              <>
                <p className="label-caps mt-4 mb-2 px-0.5">{plural(waiting.length, "review")} waiting on you</p>
                <ReviewsList reviews={waiting} loading={false} />
              </>
            )}
            {openList.length > 0 && (
              <>
                <p className="label-caps mt-4 mb-2 px-0.5">{plural(openList.length, "question")} waiting · oldest first</p>
                <ul className="space-y-3" aria-label="Open questions">
                  {openList.map((q) => (
                    <QuestionCard key={q.id} q={q} highlight={focus === q.id} onOpen={(id, mode, option) => setSheet({ id, mode, option })} />
                  ))}
                </ul>
                <MoreButton more={open} what="questions" />
              </>
            )}
          </>
        )
      ) : shown.length === 0 && !closed.isLoading ? (
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
      {(filter === "answered" || filter === "dismissed") && <MoreButton more={closed} what="questions" />}

      <QuestionSheet
        key={sheet.id ? `${sheet.id}/${sheet.option ?? ""}` : "none"}
        state={sheet}
        setMode={(mode) => setSheet((st) => ({ ...st, mode }))}
        onClose={() => setSheet((st) => ({ ...st, id: null }))}
      />
    </div>
  );
}

/** "Show more" under a paged list. */
function MoreButton({ more, what }: { more: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage: () => unknown }; what: string }) {
  if (!more.hasNextPage) return null;
  return (
    <div className="mt-4 flex justify-center">
      <Button type="button" variant="outline" disabled={more.isFetchingNextPage} onClick={() => void more.fetchNextPage()}>
        {more.isFetchingNextPage && <Loader2 className="animate-spin" />}
        Show more {what}
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------- ask

type Row = { key: number; label: string; description: string };
type AskKind = "free" | "cards" | "single" | "multiple";

const ASK_KINDS: { id: AskKind; label: string; hint: string; icon: typeof Pencil }[] = [
  { id: "free", label: "Free text", hint: "They write the answer.", icon: Pencil },
  { id: "cards", label: "Cards", hint: "Quick choices with short labels. Best for 2 to 4 options.", icon: LayoutGrid },
  { id: "single", label: "Pick one", hint: "A radio list. Each option can have a line of context.", icon: CircleDot },
  { id: "multiple", label: "Pick several", hint: "Checkboxes, with an optional minimum and maximum.", icon: ListChecks },
];

let rowKey = 0;
const newRow = (label = ""): Row => ({ key: ++rowKey, label, description: "" });

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
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [kind, setKind] = useState<AskKind>("free");
  const [rows, setRows] = useState<Row[]>([]);
  const [allowOther, setAllowOther] = useState(false);
  const [min, setMin] = useState("");
  const [max, setMax] = useState("");
  const [taskId, setTaskId] = useState("");
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setTitle("");
      setDescription("");
      setKind("free");
      setRows([]);
      setAllowOther(false);
      setMin("");
      setMax("");
      setTaskId(task ?? "");
      setTo("");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function pickKind(k: AskKind) {
    setKind(k);
    if (k !== "free" && rows.length < 2) setRows((r) => [...r, ...Array.from({ length: 2 - r.length }, () => newRow())]);
  }
  const patch = (key: number, p: Partial<Row>) => setRows((r) => r.map((x) => (x.key === key ? { ...x, ...p } : x)));
  const move = (i: number, d: -1 | 1) =>
    setRows((r) => {
      const j = i + d;
      if (j < 0 || j >= r.length) return r;
      const c = [...r];
      [c[i], c[j]] = [c[j]!, c[i]!];
      return c;
    });

  const filled = rows.filter((r) => r.label.trim());
  const need = kind === "cards" ? 2 : kind === "free" ? 0 : 1;
  const hint = !title.trim() ? "Give it a title" : filled.length < need ? `Add at least ${need} ${need === 1 ? "choice" : "choices"}` : null;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (hint) return;
    setBusy(true);
    try {
      const q = await call("questionAsk", {
        title: title.trim(),
        description: description.trim() || null,
        project: potKey,
        task: taskId.trim().toUpperCase() || null,
        to: to.trim() || null,
        kind,
        choices: kind === "free" ? null : JSON.stringify(filled.map((r) => ({ label: r.label.trim(), ...(kind !== "cards" && r.description.trim() ? { description: r.description.trim() } : {}) }))),
        allowOther: kind === "free" ? null : allowOther,
        min: kind === "multiple" && min ? Number(min) : null,
        max: kind === "multiple" && max ? Number(max) : null,
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
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Ask a question in {potKey}</DialogTitle>
          <DialogDescription>It goes into the pot’s pool for its people (or one person or cook) to answer.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="aq-title">Question</Label>
            <Input id="aq-title" autoFocus required maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="One short line, e.g. Round prices up or down?" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="aq-desc">Context (optional, markdown)</Label>
            <Textarea id="aq-desc" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What the answerer needs to know to decide without reading the code." />
          </div>

          <fieldset className="space-y-2">
            <legend className="mb-1.5 text-sm leading-none font-medium">How should it be answered?</legend>
            <div role="radiogroup" aria-label="Answer type" className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {ASK_KINDS.map((k) => (
                <label
                  key={k.id}
                  className={cn(
                    "flex min-h-14 cursor-pointer flex-col items-center justify-center gap-1 rounded-xl border px-2 py-2 text-center text-[13px] font-medium transition-colors has-[:focus-visible]:ring-[3px] has-[:focus-visible]:ring-ring/50",
                    kind === k.id ? "border-primary bg-primary/10 shadow-[inset_0_0_0_1px_var(--color-primary)]" : "border-input hover:border-foreground/40",
                  )}
                >
                  <input type="radio" name="aq-kind" className="sr-only" checked={kind === k.id} onChange={() => pickKind(k.id)} />
                  <k.icon className="size-4" aria-hidden="true" />
                  {k.label}
                </label>
              ))}
            </div>
            <p className="text-[12.5px] text-muted-foreground">{ASK_KINDS.find((k) => k.id === kind)!.hint}</p>
          </fieldset>

          {kind !== "free" && (
            <div className="space-y-2.5">
              <p className="text-sm leading-none font-medium">Choices</p>
              <ul className="space-y-2">
                {rows.map((r, i) => (
                  <li key={r.key} className="flex flex-wrap items-start gap-1.5 sm:flex-nowrap rounded-xl border border-border bg-muted/30 p-2">
                    <span className="mt-2 w-4 shrink-0 text-center font-mono text-[11px] text-muted-foreground">{i + 1}</span>
                    <div className="min-w-0 flex-1 basis-[calc(100%-2rem)] space-y-1.5 sm:basis-0">
                      <Input aria-label={`Choice ${i + 1} label`} value={r.label} maxLength={120} onChange={(e) => patch(r.key, { label: e.target.value })} placeholder={kind === "cards" ? "Short label, e.g. Yes" : "Label"} className="h-9" />
                      {kind !== "cards" && (
                        <Input aria-label={`Choice ${i + 1} description`} value={r.description} maxLength={300} onChange={(e) => patch(r.key, { description: e.target.value })} placeholder="One line of context (optional)" className="h-8 text-[13px]" />
                      )}
                    </div>
                    <div className="flex w-full shrink-0 justify-end sm:w-auto">
                      <Button type="button" variant="ghost" size="icon-sm" disabled={i === 0} onClick={() => move(i, -1)} aria-label={`Move choice ${i + 1} up`}>
                        <ArrowUp />
                      </Button>
                      <Button type="button" variant="ghost" size="icon-sm" disabled={i === rows.length - 1} onClick={() => move(i, 1)} aria-label={`Move choice ${i + 1} down`}>
                        <ArrowDown />
                      </Button>
                      <Button type="button" variant="ghost" size="icon-sm" onClick={() => setRows((x) => x.filter((y) => y.key !== r.key))} aria-label={`Remove choice ${i + 1}`}>
                        <Trash2 />
                      </Button>
                    </div>
                  </li>
                ))}
              </ul>
              <Button type="button" variant="outline" size="sm" onClick={() => setRows((r) => [...r, newRow()])} disabled={rows.length >= 12}>
                <Plus /> Add a choice
              </Button>
              <div className="flex items-center justify-between gap-3 rounded-xl border border-border px-3 py-2.5">
                <div className="min-w-0 flex-1">
                  <Label htmlFor="aq-other" className="block font-medium">
                    Allow “Other…”
                  </Label>
                  <p className="mt-0.5 text-[12.5px] text-muted-foreground">The answerer can write their own answer too.</p>
                </div>
                <Switch id="aq-other" checked={allowOther} onCheckedChange={setAllowOther} />
              </div>
              {kind === "multiple" && (
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="aq-min">At least (optional)</Label>
                    <Input id="aq-min" type="number" inputMode="numeric" min={0} max={12} value={min} onChange={(e) => setMin(e.target.value)} />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="aq-max">At most (optional)</Label>
                    <Input id="aq-max" type="number" inputMode="numeric" min={1} max={13} value={max} onChange={(e) => setMax(e.target.value)} />
                  </div>
                </div>
              )}
            </div>
          )}

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
          <DialogFooter className="items-center gap-2">
            {hint && <p className="mr-auto text-[12.5px] text-muted-foreground">{hint}</p>}
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !!hint}>
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
      <h3 className="font-display text-[16px] leading-snug font-extrabold text-balance">{data.title}</h3>
      <Description q={data} />
      <div className="rounded-lg border border-border bg-card p-3">
        <AnswerControls q={data} compact idPrefix={`parked-${data.id}`} />
      </div>
    </section>
  );
}
