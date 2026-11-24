import { useEffect, useMemo, useState, type FormEvent } from "react";
import type { ReviewSummary, ReviewView, TaskDetail } from "@corntchap/api";
import { ClipboardCheck, CircleCheck, CircleX, Eye, Loader2, MessageSquare, Undo2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useProcCall, useProjects, useTeamChefs } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useSession } from "@/lib/session";
import { projectOfTask, useOpenTask } from "@/lib/nav";
import { cn } from "@/lib/utils";
import { Avatar, EmptyState, IdChip, SectionLabel } from "./bits";
import { Markdown } from "./markdown";

/**
 * A review is a cook or a person looking at a task's work (README: "Questions to a cook, and
 * reviews"). It is not a lane: the task keeps its status. GitHub's own reviews on a pull request
 * stay on the pull request (forge-prs.tsx); these are shown apart and named "Review".
 */

const chip = "inline-flex h-6 shrink-0 items-center gap-1 rounded-full border px-2 text-[11.5px] font-medium leading-none";

type Tone = { icon: typeof Eye; label: string; short: string; cls: string };

/** How a review state looks: the long label (task sheet) and the short one (board chip). */
export function toneOf(state: string): Tone {
  switch (state) {
    case "approved":
      return { icon: CircleCheck, label: "Approved", short: "approved", cls: "border-success/40 bg-success/10 text-success" };
    case "changes_requested":
      return { icon: CircleX, label: "Changes requested", short: "changes", cls: "border-destructive/40 bg-destructive/10 text-destructive" };
    case "commented":
      return { icon: MessageSquare, label: "Commented", short: "commented", cls: "border-border bg-muted/60 text-muted-foreground" };
    case "cancelled":
      return { icon: Undo2, label: "Withdrawn", short: "withdrawn", cls: "border-border bg-muted/60 text-muted-foreground" };
    default:
      return { icon: Eye, label: "Waiting for review", short: "in review", cls: "border-corn/50 bg-corn/15 text-corn-text" };
  }
}

/** The small chip on a board card or list row: the latest review round. Nothing without one. */
export function ReviewChip({ review }: { review?: ReviewSummary | null | undefined }) {
  if (!review) return null;
  const t = toneOf(review.state);
  const who = review.state === "requested" ? (review.reviewer ? `from ${review.reviewer}` : "from the team") : review.reviewed_by ? `by ${review.reviewed_by}` : "";
  const title = `Review ${review.id}, round ${review.round}: ${t.label.toLowerCase()}${who ? ` ${who}` : ""}`;
  return (
    <span className={cn(chip, "px-1.5", t.cls)} title={title} role="img" aria-label={title}>
      <t.icon className="size-3" aria-hidden="true" />
      <span className="text-[11px]">{t.short}</span>
    </span>
  );
}

/** Whether `me` may give the verdict: the named reviewer, or anyone but the requester when none is named. */
export function canReview(r: { state: string; reviewer?: string | null | undefined; requested_by?: string | null | undefined }, me: string): boolean {
  return r.state === "requested" && (r.reviewer ? r.reviewer === me : r.requested_by !== me);
}

/** The three verdict buttons with a text box: approve, request changes, comment. */
function VerdictForm({ review, onDone }: { review: ReviewView; onDone?: () => void }) {
  const call = useProcCall();
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  async function give(verdict: "approve" | "changes" | "comment") {
    if (verdict !== "approve" && !body.trim()) {
      toast.error(verdict === "changes" ? "Say what to change" : "Write your comment first");
      return;
    }
    setBusy(verdict);
    try {
      await call("reviewVerdict", { id: review.id, verdict, body: body.trim() || null });
      toast.success(verdict === "approve" ? `${review.task} approved` : verdict === "changes" ? "Changes requested" : "Comment sent");
      setBody("");
      onDone?.();
    } catch (err) {
      toast.error("Could not send your verdict", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="mt-3 space-y-2.5 border-t border-border pt-3">
      <Label htmlFor={`rv-${review.id}`} className="sr-only">
        Your review of {review.task}
      </Label>
      <Textarea
        id={`rv-${review.id}`}
        rows={3}
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="What you found. Required for changes and comments; the cook is woken with it."
      />
      <div className="flex flex-wrap gap-2">
        <Button className="max-sm:h-11 max-sm:flex-1" disabled={!!busy} onClick={() => void give("approve")}>
          {busy === "approve" ? <Loader2 className="animate-spin" /> : <CircleCheck />} Approve
        </Button>
        <Button className="max-sm:h-11 max-sm:flex-1" variant="outline" disabled={!!busy} onClick={() => void give("changes")}>
          {busy === "changes" ? <Loader2 className="animate-spin" /> : <CircleX />} Request changes
        </Button>
        <Button className="max-sm:h-11 max-sm:flex-1" variant="ghost" disabled={!!busy} onClick={() => void give("comment")}>
          {busy === "comment" ? <Loader2 className="animate-spin" /> : <MessageSquare />} Comment
        </Button>
      </div>
    </div>
  );
}

function Who({ name }: { name: string }) {
  return (
    <span className="inline-flex items-center gap-1">
      <Avatar name={name} className="size-4 rounded-[5px] text-[9px]" />
      {name}
    </span>
  );
}

/** One round: who asked whom, what to look at, the verdict, and the buttons when it waits on you. */
function ReviewStrip({ r, showTask, onCancel, cancelling }: { r: ReviewView; showTask?: boolean; onCancel?: () => void; cancelling?: boolean }) {
  const me = useSession();
  const now = useNow();
  const open = useOpenTask();
  const t = toneOf(r.state);
  const mine = canReview(r, me.name);
  return (
    <li className={cn("rounded-lg border bg-card p-3", mine ? "border-corn/60" : "border-border")}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        <span className={cn(chip, t.cls)}>
          <t.icon className="size-3.5" aria-hidden="true" /> {t.label}
        </span>
        <span className="font-mono text-[11.5px] text-muted-foreground">
          {r.id} · round {r.round}
        </span>
        <span className="ml-auto text-[12px] text-muted-foreground">{fmtAgo(r.decided_at ?? r.created_at, now)}</span>
      </div>
      {showTask && (
        <button type="button" onClick={() => open(r.task)} className="mt-2 flex max-w-full items-baseline gap-2 text-left max-md:py-1" title={`Open ${r.task}`}>
          <IdChip id={r.task} className="text-[11.5px]" />
          <span className="truncate text-[14.5px] font-medium underline-offset-2 hover:underline">{r.task_title}</span>
        </button>
      )}
      <p className="mt-2 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[12.5px] text-muted-foreground">
        {r.requested_by && <Who name={r.requested_by} />} asked {r.reviewer ? <Who name={r.reviewer} /> : <span>anyone on the team</span>}
        {r.reviewed_by && r.state !== "requested" && r.state !== "cancelled" && (
          <>
            · answered by <Who name={r.reviewed_by} />
          </>
        )}
      </p>
      {r.note.trim() && <Markdown className="mt-2 text-[13.5px] [&_p]:leading-relaxed">{r.note}</Markdown>}
      {r.body && (
        <div className="mt-2.5 rounded-md border-l-2 border-border bg-muted/40 px-3 py-2">
          <Markdown className="text-[13.5px] [&_p]:leading-relaxed">{r.body}</Markdown>
        </div>
      )}
      {mine && <VerdictForm review={r} />}
      {r.state === "requested" && !mine && me.name === r.requested_by && onCancel && (
        <div className="mt-2 flex justify-end">
          <Button variant="ghost" size="sm" className="text-muted-foreground max-md:h-11" disabled={cancelling} onClick={onCancel}>
            {cancelling ? <Loader2 className="animate-spin" /> : <Undo2 />} Withdraw
          </Button>
        </div>
      )}
    </li>
  );
}

/** A list of reviews with their task (the Questions page: waiting on you, and all of them). */
export function ReviewsList({ reviews, loading }: { reviews: ReviewView[]; loading: boolean }) {
  const call = useProcCall();
  const [busy, setBusy] = useState<string | null>(null);
  if (loading) return <Skeleton className="mt-4 h-24 w-full" />;
  if (reviews.length === 0) {
    return (
      <EmptyState band={false} className="mt-4" title="No reviews yet">
        A cook asks for one when its work is ready to look at (request_review), and you can ask from a task’s sheet. They show up here, and the ones waiting on you come first.
      </EmptyState>
    );
  }
  async function cancel(r: ReviewView) {
    setBusy(r.id);
    try {
      await call("reviewCancel", { id: r.id });
      toast.success("Review request withdrawn");
    } catch (err) {
      toast.error("Could not withdraw it", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }
  return (
    <ul className="space-y-2.5" aria-label="Reviews">
      {reviews.map((r) => (
        <ReviewStrip key={r.id} r={r} showTask onCancel={() => void cancel(r)} cancelling={busy === r.id} />
      ))}
    </ul>
  );
}

/** Ask a cook or person of the pot's team to review a task. */
export function RequestReviewDialog({ task, open, onClose, suggestion }: { task: string; open: boolean; onClose: () => void; suggestion?: string }) {
  const call = useProcCall();
  const me = useSession();
  const pot = projectOfTask(task);
  const projects = useProjects();
  const team = projects.data?.find((p) => p.key === pot)?.team ?? null;
  const chefs = useTeamChefs(team, { enabled: open });
  const options = useMemo(() => (chefs.data ?? []).filter((c) => c.name !== me.name), [chefs.data, me.name]);
  const [reviewer, setReviewer] = useState("__anyone");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setReviewer("__anyone");
      setNote(suggestion ?? "");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await call("reviewRequest", { task, reviewer: reviewer === "__anyone" ? null : reviewer, note: note.trim() || null });
      toast.success(`${r.id} asked${r.reviewer ? ` of ${r.reviewer}` : ""}`);
      onClose();
    } catch (err) {
      toast.error("Could not ask for a review", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Request a review of {task}</DialogTitle>
          <DialogDescription>A cook is woken with the task and your note; a person finds it under Questions &gt; Reviews. The task keeps its status.</DialogDescription>
        </DialogHeader>
        <form onSubmit={(e) => void submit(e)} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="rr-who">Who should look</Label>
            <Select value={reviewer} onValueChange={(v) => v && setReviewer(v)}>
              <SelectTrigger id="rr-who" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__anyone">Anyone on the team</SelectItem>
                {options.map((c) => (
                  <SelectItem key={c.name} value={c.name}>
                    {c.name} <span className="text-muted-foreground">{c.kind === "agent" ? "cook" : "person"}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="rr-note">What to look at (optional)</Label>
            <Textarea id="rr-note" rows={3} value={note} onChange={(e) => setNote(e.target.value)} placeholder="The pull request address, the branch, what to check." />
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="animate-spin" /> : <ClipboardCheck />} Ask for it
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** The task sheet's review block: every round, newest first, the verdict buttons when it waits on you, and the request action. */
export function TaskReviews({ task }: { task: TaskDetail }) {
  const call = useProcCall();
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const reviews = useMemo(() => [...(task.reviews ?? [])].reverse(), [task.reviews]);
  if (task.status === "cancelled" && reviews.length === 0) return null;
  const suggestion = task.prs?.[0]?.url ?? "";

  async function cancel(r: ReviewView) {
    setBusy(r.id);
    try {
      await call("reviewCancel", { id: r.id });
      toast.success("Review request withdrawn");
    } catch (err) {
      toast.error("Could not withdraw it", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <section aria-labelledby="reviews">
      <div className="mb-2 flex items-center justify-between gap-2">
        <SectionLabel>
          <span id="reviews">Review</span>
        </SectionLabel>
        {task.status !== "cancelled" && (
          <Button variant="ghost" size="sm" className="text-muted-foreground max-md:h-11" onClick={() => setAsking(true)}>
            <ClipboardCheck /> Request review
          </Button>
        )}
      </div>
      {reviews.length === 0 ? (
        <p className="text-muted-foreground">Nobody has looked at this work yet. A cook or a person from the team can review it; GitHub’s own reviews show with the pull request above.</p>
      ) : (
        <ul className="space-y-2">
          {reviews.map((r) => (
            <ReviewStrip key={r.id} r={r} onCancel={() => void cancel(r)} cancelling={busy === r.id} />
          ))}
        </ul>
      )}
      <RequestReviewDialog task={task.id} open={asking} onClose={() => setAsking(false)} suggestion={suggestion} />
    </section>
  );
}
