import { useEffect, useState, type FormEvent } from "react";
import { Link } from "react-router";
import type { QuestionTaskView, QuestionView, TaskDetail, TaskQuestionView, TaskView } from "@corntchap/api";
import { Hourglass, Link2, Loader2, Unlink } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { useQuestionsSource, useTasksSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { projectOfTask, useOpenTask } from "@/lib/nav";
import { cn } from "@/lib/utils";
import { IdChip, SectionLabel } from "./bits";
import { Markdown } from "./markdown";

/**
 * A question can be linked to tasks: whoever linked it (and the holder and owner of the task) is woken when it is
 * answered, and a link that blocks keeps the task out of the ready column until then (README: "Link a question to
 * a task"). The same actions exist as the MCP tools `link_question` and `unlink_question`.
 */

const chip = "inline-flex h-6 shrink-0 items-center gap-1 rounded-full border px-2 text-[11.5px] font-medium leading-none";

const questionPath = (pot: string, id: string) => `/pots/${pot}/questions?question=${encodeURIComponent(id)}`;

/** The small marker on a board card or list row: the task waits on an open question. */
export function QuestionBlockedChip({ task }: { task: Pick<TaskView, "question_blocked"> }) {
  if (!task.question_blocked) return null;
  return (
    <span className={cn(chip, "border-corn/50 bg-corn/15 px-1.5 text-corn-text")} title="Waiting on a question: it is not ready until the question is answered" role="img" aria-label="Waiting on a question">
      <Hourglass className="size-3" aria-hidden="true" />
      <span className="text-[11px]">question</span>
    </span>
  );
}

function StatusChip({ status }: { status: string }) {
  const cls =
    status === "answered"
      ? "border-success/40 bg-success/10 text-success"
      : status === "dismissed"
        ? "border-border bg-muted/60 text-muted-foreground"
        : "border-corn/50 bg-corn/15 text-corn-text";
  return <span className={cn(chip, cls)}>{status}</span>;
}

function BlocksBadge({ blocks, open }: { blocks: boolean; open: boolean }) {
  if (!blocks) return null;
  return (
    <span className={cn(chip, open ? "border-destructive/40 bg-destructive/10 text-destructive" : "border-border bg-muted/60 text-muted-foreground")}>
      <Hourglass className="size-3" aria-hidden="true" />
      {open ? "blocks the task" : "was blocking"}
    </span>
  );
}

function BlocksChoice({ blocks, onChange }: { blocks: boolean; onChange: (b: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5">
      <Checkbox checked={blocks} onCheckedChange={(c) => onChange(c === true)} className="mt-0.5" />
      <span>
        <span className="font-medium">The task waits on the answer</span>
        <span className="block text-[12.5px] text-muted-foreground">It is not ready (nobody is handed it) until the question is answered or dismissed.</span>
      </span>
    </label>
  );
}

/** Link an open question of the task's pot to the task. */
export function LinkQuestionDialog({ task, open, onClose }: { task: string; open: boolean; onClose: () => void }) {
  const call = useProcCall();
  const pot = projectOfTask(task);
  const query = useStoreValue("");
  const source = useQuestionsSource(pot, { status: "open", q: query.value, enabled: open });
  const [question, setQuestion] = useState<string | null>(null);
  const [blocks, setBlocks] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setQuestion(null);
      setBlocks(true);
      query.set("");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!question) return;
    setBusy(true);
    try {
      await call("questionLink", { question, task, blocks });
      toast.success(`${question} linked to ${task}`);
      onClose();
    } catch (err) {
      toast.error("Could not link it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Link a question to {task}</DialogTitle>
          <DialogDescription>The cooks of the task are woken when the question is answered. Pick one of the pot’s open questions.</DialogDescription>
        </DialogHeader>
        <form onSubmit={(e) => void submit(e)} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="lq-question">Question</Label>
            <VirtualCombobox<QuestionView>
              id="lq-question"
              label="Question"
              placeholder="Search the open questions"
              source={source}
              onQuery={query.set}
              itemKey={(q) => q.id}
              itemText={(q) => `${q.id} ${q.title}`}
              value={question}
              onChange={setQuestion}
              renderItem={(q) => (
                <span className="flex min-w-0 items-baseline gap-2">
                  <IdChip id={q.id} className="shrink-0 text-[11.5px]" />
                  <span className="truncate">{q.title}</span>
                </span>
              )}
            />
          </div>
          <BlocksChoice blocks={blocks} onChange={setBlocks} />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !question}>
              {busy ? <Loader2 className="animate-spin" /> : <Link2 />} Link it
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Link a question to one of the pot's tasks. */
export function LinkTaskDialog({ question, open, onClose }: { question: string; open: boolean; onClose: () => void }) {
  const call = useProcCall();
  const pot = projectOfTask(question);
  const query = useStoreValue("");
  const source = useTasksSource(pot, { bucket: "active", q: query.value, enabled: open });
  const [task, setTask] = useState<string | null>(null);
  const [blocks, setBlocks] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setTask(null);
      setBlocks(true);
      query.set("");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!task) return;
    setBusy(true);
    try {
      await call("questionLink", { question, task, blocks });
      toast.success(`${question} linked to ${task}`);
      onClose();
    } catch (err) {
      toast.error("Could not link it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Link {question} to a task</DialogTitle>
          <DialogDescription>The cooks of the task are woken when this question is answered.</DialogDescription>
        </DialogHeader>
        <form onSubmit={(e) => void submit(e)} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="lt-task">Task</Label>
            <VirtualCombobox<TaskView>
              id="lt-task"
              label="Task"
              placeholder="Search the pot’s tasks"
              source={source}
              onQuery={query.set}
              itemKey={(t) => t.id}
              itemText={(t) => `${t.id} ${t.title}`}
              value={task}
              onChange={setTask}
              renderItem={(t) => (
                <span className="flex min-w-0 items-baseline gap-2">
                  <IdChip id={t.id} className="shrink-0 text-[11.5px]" />
                  <span className="truncate">{t.title}</span>
                </span>
              )}
            />
          </div>
          <BlocksChoice blocks={blocks} onChange={setBlocks} />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !task}>
              {busy ? <Loader2 className="animate-spin" /> : <Link2 />} Link it
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function useUnlink() {
  const call = useProcCall();
  const [busy, setBusy] = useState<string | null>(null);
  async function unlink(question: string, task: string) {
    setBusy(`${question}|${task}`);
    try {
      await call("questionUnlink", { question, task });
      toast.success(`${question} unlinked from ${task}`);
    } catch (err) {
      toast.error("Could not unlink it", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }
  return { unlink, busy };
}

function QuestionRow({ q, task, onUnlink, busy }: { q: TaskQuestionView; task: string; onUnlink: () => void; busy: boolean }) {
  return (
    <li className="space-y-1.5 rounded-lg border border-border bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <Link to={questionPath(projectOfTask(q.id), q.id)} className="font-mono text-[12px] font-bold text-foreground/80 hover:underline">
          {q.id}
        </Link>
        <StatusChip status={q.status} />
        <BlocksBadge blocks={q.blocks} open={q.status === "open"} />
        {q.implicit && <span className="text-[12px] text-muted-foreground">asked on this task</span>}
        {!q.implicit && (
          <Button variant="ghost" size="sm" className="ml-auto h-7 text-muted-foreground max-md:h-11" onClick={onUnlink} disabled={busy} aria-label={`Unlink ${q.id} from ${task}`}>
            {busy ? <Loader2 className="animate-spin" /> : <Unlink />} <span className="max-sm:sr-only">Unlink</span>
          </Button>
        )}
      </div>
      <p className="text-[14px] leading-snug font-medium">{q.title}</p>
      {q.answer && (
        <div className="rounded-md border border-success/30 bg-success/10 px-2.5 py-1.5">
          <Markdown className="text-[13px]">{q.answer}</Markdown>
          {q.answered_by && (
            <p className="text-[11.5px] text-muted-foreground">
              {q.status === "dismissed" ? "dismissed" : "answered"} by {q.answered_by}
            </p>
          )}
        </div>
      )}
    </li>
  );
}

/** The task sheet's "Questions" block: every linked question with its status, whether it blocks, and its answer. */
export function TaskQuestions({ task }: { task: TaskDetail }) {
  const [linking, setLinking] = useState(false);
  const { unlink, busy } = useUnlink();
  const questions = task.questions ?? [];
  const closed = task.status === "done" || task.status === "cancelled";
  if (closed && questions.length === 0) return null;
  return (
    <section aria-labelledby="task-questions">
      <div className="mb-2 flex items-center justify-between gap-2">
        <SectionLabel>
          <span id="task-questions">Questions</span>
        </SectionLabel>
        {!closed && (
          <Button variant="ghost" size="sm" className="text-muted-foreground max-md:h-11" onClick={() => setLinking(true)}>
            <Link2 /> Link a question
          </Button>
        )}
      </div>
      {questions.length === 0 ? (
        <p className="text-muted-foreground">No question is linked. Link one the task waits on, and its cooks are woken when it is answered.</p>
      ) : (
        <ul className="space-y-2">
          {questions.map((q) => (
            <QuestionRow key={q.id} q={q} task={task.id} busy={busy === `${q.id}|${task.id}`} onUnlink={() => void unlink(q.id, task.id)} />
          ))}
        </ul>
      )}
      <LinkQuestionDialog task={task.id} open={linking} onClose={() => setLinking(false)} />
    </section>
  );
}

function TaskLinkChip({ t, onUnlink, busy }: { t: QuestionTaskView; onUnlink: () => void; busy: boolean }) {
  const open = useOpenTask();
  return (
    <span className="inline-flex items-center overflow-hidden rounded-md border border-border bg-muted/50">
      <button type="button" onClick={() => open(t.id)} title={`Open ${t.id}: ${t.title}`} className="inline-flex items-center gap-1.5 px-1.5 py-px hover:bg-muted focus-visible:bg-muted">
        <IdChip id={t.id} className="text-[11.5px]" />
        {t.blocks && <Hourglass className="size-3 text-corn-text" aria-label="blocks the task" />}
      </button>
      {!t.implicit && (
        <button
          type="button"
          onClick={onUnlink}
          disabled={busy}
          aria-label={`Unlink ${t.id}`}
          title={`Unlink ${t.id}`}
          className="border-l border-border px-1.5 py-px text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:bg-muted"
        >
          {busy ? <Loader2 className="size-3 animate-spin" /> : <Unlink className="size-3" />}
        </button>
      )}
    </span>
  );
}

/** On a question card: the tasks it is linked to as chips, and "Link to a task" while it is open. */
export function QuestionTasks({ q, open = true }: { q: QuestionView; open?: boolean }) {
  const [linking, setLinking] = useState(false);
  const { unlink, busy } = useUnlink();
  const tasks = q.tasks ?? [];
  if (!open && tasks.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label={`Tasks linked to ${q.id}`}>
      {tasks.map((t) => (
        <TaskLinkChip key={t.id} t={t} busy={busy === `${q.id}|${t.id}`} onUnlink={() => void unlink(q.id, t.id)} />
      ))}
      {open && (
        <Button variant="ghost" size="sm" className="h-7 px-2 text-[12.5px] text-muted-foreground max-md:h-11" onClick={() => setLinking(true)}>
          <Link2 /> Link to a task
        </Button>
      )}
      {open && <LinkTaskDialog question={q.id} open={linking} onClose={() => setLinking(false)} />}
    </div>
  );
}
