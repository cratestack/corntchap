import { useEffect, useMemo, useState, type FormEvent } from "react";
import type { TaskDetail } from "@corntchap/api";
import { Ban, CheckCheck, Loader2, Pencil, Send, Unlock } from "lucide-react";
import { toast } from "sonner";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useProc, useProcCall, useV1Call } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { BUCKETS, bucketOf, fmtAgo, useNow, taskNote } from "@/lib/format";
import { useSession } from "@/lib/session";
import { useOpenTask } from "@/lib/nav";
import { Avatar, BUCKET_DOT, IdChip, LabelChip, LeaseTimer, ProgressBar, SectionLabel } from "./bits";
import { Markdown } from "./markdown";
import { TagInput } from "./tag-input";
import { PRIORITIES } from "./new-task";
import { cn } from "@/lib/utils";

export function TaskSheet({ id, onClose }: { id: string | null; onClose: () => void }) {
  return (
    <Sheet open={!!id} onOpenChange={(o) => !o && onClose()}>
      <SheetContent
        side="right"
        className="w-full gap-0 overflow-y-auto p-0 sm:max-w-[34rem]"
        aria-describedby="task-sheet-desc"
      >
        <SheetTitle className="sr-only">Task {id}</SheetTitle>
        <SheetDescription id="task-sheet-desc" className="sr-only">
          Checklist, comments and actions for the task
        </SheetDescription>
        {id && <TaskDetailBody id={id} />}
      </SheetContent>
    </Sheet>
  );
}

function TaskDetailBody({ id }: { id: string }) {
  const q = useProc("taskDetail", { id });
  const me = useSession();
  const call = useProcCall();
  const v1 = useV1Call();
  const open = useOpenTask();
  const now = useNow();
  const [editing, setEditing] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [serving, setServing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => setEditing(false), [id]);

  const t = q.data;

  if (q.error && !t) {
    return (
      <div className="p-8">
        <h2 className="text-2xl">That task is not in the pot</h2>
        <p className="mt-2 text-muted-foreground">{errorMessage(q.error)}</p>
      </div>
    );
  }
  if (!t) {
    return (
      <div className="space-y-4 p-6" aria-busy="true" aria-label="Loading task">
        <Skeleton className="h-4 w-20" />
        <Skeleton className="h-8 w-3/4" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  const bucket = bucketOf(t);
  const mine = t.claimed_by === me.name && !!t.lease_live;
  const closed = t.status === "done" || t.status === "cancelled";
  const done = t.total_items - t.open_items;
  const note = taskNote(t as never);
  const canServe = !closed && (bucket === "ready" || mine);

  async function run<T>(label: string, fn: () => Promise<T>, ok?: string) {
    setBusy(label);
    try {
      await fn();
      if (ok) toast.success(ok);
    } catch (e) {
      toast.error(`Could not ${label}`, { description: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  }

  const tick = (position: number, checked: boolean) =>
    // Humans tick without a lease: the generated checkItem procedure needs an epoch (agents' path), so this uses /api/v1.
    run("tick that item", () => v1("POST", `/tasks/${t.id}/check`, { position, done: checked }));

  return (
    <div className="flex min-h-full flex-col">
      <header className="border-b border-border px-6 pt-6 pb-4">
        <div className="flex flex-wrap items-center gap-2.5 pr-8">
          <IdChip id={t.id} className="text-[13px]" />
          <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-muted/60 px-2 py-0.5 text-[11.5px] font-medium">
            <span className={cn("size-2 rounded-full", BUCKET_DOT[bucket], bucket === "claimed" && "live-dot")} aria-hidden="true" />
            {BUCKETS.find((b) => b.id === bucket)?.label}
          </span>
          {t.priority !== 0 && (
            <span className="font-mono text-[10.5px] uppercase tracking-wider text-muted-foreground">
              {PRIORITIES.find((p) => p.v === Math.max(-1, Math.min(2, t.priority)))?.label} priority
            </span>
          )}
        </div>
        {editing ? null : <h2 className="mt-2.5 text-[26px] leading-[1.15]">{t.title}</h2>}
        {!editing && (
          <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
            {t.labels.map((l) => (
              <LabelChip key={l}>{l}</LabelChip>
            ))}
            {t.claimed_by && (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
                <Avatar name={t.claimed_by} className="size-5 text-[10px]" /> {t.claimed_by}
              </span>
            )}
            {t.status === "claimed" && <LeaseTimer task={t} />}
          </div>
        )}
        {note && !editing && <p className="mt-2 text-[12.5px] text-muted-foreground">{note}</p>}

        {!editing && (
          <div className="mt-4 flex flex-wrap gap-2">
            {canServe && (
              <Button onClick={() => setServing(true)} disabled={!!busy}>
                <CheckCheck /> Serve
              </Button>
            )}
            {t.status === "blocked" && (
              <Button
                variant="outline"
                disabled={!!busy}
                onClick={() => run("unblock", () => call("unblockTask", { id: t.id }), `${t.id} is back in the pot`)}
              >
                {busy === "unblock" ? <Loader2 className="animate-spin" /> : <Unlock />} Unblock
              </Button>
            )}
            {!closed && (
              <Button variant="outline" onClick={() => setEditing(true)} disabled={!!busy}>
                <Pencil /> Edit
              </Button>
            )}
            {!closed && (
              <Button variant="ghost" className="text-destructive hover:text-destructive" onClick={() => setConfirmCancel(true)} disabled={!!busy}>
                <Ban /> Cancel
              </Button>
            )}
          </div>
        )}
      </header>

      {editing ? (
        <EditForm
          t={t}
          onDone={() => setEditing(false)}
          save={(args) => call("taskEdit", { id: t.id, ...args })}
        />
      ) : (
        <div className="flex-1 space-y-7 px-6 py-5">
          <section aria-label="Description">
            {t.body.trim() ? (
              <Markdown>{t.body}</Markdown>
            ) : (
              <p className="text-muted-foreground">No description. Edit the task to add one.</p>
            )}
          </section>

          {t.result && closed && (
            <section className="rounded-lg border border-success/40 bg-success/10 p-3.5">
              <SectionLabel className="!text-success">{t.status === "done" ? "Served" : "Result"}</SectionLabel>
              <p className="mt-1">{t.result}</p>
            </section>
          )}

          <section aria-labelledby="ck">
            <div className="mb-2.5 flex items-center justify-between">
              <SectionLabel>
                <span id="ck">Checklist</span>
              </SectionLabel>
              {t.total_items > 0 && <ProgressBar done={done} total={t.total_items} />}
            </div>
            {t.items.length === 0 ? (
              <p className="text-muted-foreground">No checklist items.</p>
            ) : (
              <ul className="space-y-0.5">
                {t.items.map((it) => (
                  <li key={it.position}>
                    <label className="flex cursor-pointer items-start gap-3 rounded-lg px-2 py-1.5 hover:bg-muted/60">
                      <Checkbox
                        checked={it.done}
                        disabled={!!busy || t.status === "cancelled"}
                        onCheckedChange={(c) => void tick(it.position, c === true)}
                        aria-label={`${it.done ? "Untick" : "Tick"}: ${it.text}`}
                        className="mt-0.5"
                      />
                      <span className="min-w-0 flex-1">
                        <span className={cn("text-[13.5px]", it.done && "text-muted-foreground line-through decoration-muted-foreground/50")}>{it.text}</span>
                        {it.done && it.done_by && <span className="ml-2 font-mono text-[10.5px] text-muted-foreground">{it.done_by}</span>}
                      </span>
                      <span className="font-mono text-[10.5px] text-muted-foreground/70">{it.position}</span>
                    </label>
                  </li>
                ))}
              </ul>
            )}
          </section>

          {(t.dependencies.length > 0 || t.subtasks.length > 0 || t.parent) && (
            <section className="space-y-4">
              {t.parent && <LinkList title="Part of" items={[{ id: t.parent, title: "parent task", status: "open" }]} open={open} bare />}
              {t.dependencies.length > 0 && <LinkList title="Waits on" items={t.dependencies} open={open} />}
              {t.subtasks.length > 0 && <LinkList title="Subtasks" items={t.subtasks} open={open} />}
            </section>
          )}

          <section aria-labelledby="cm">
            <SectionLabel>
              <span id="cm">Comments · {t.comments.length}</span>
            </SectionLabel>
            <ul className="mt-2.5 space-y-3">
              {t.comments.length === 0 && <li className="text-muted-foreground">No comments yet. Leave a note for the next cook.</li>}
              {t.comments.map((c, i) => (
                <li key={i} className="flex gap-3">
                  <Avatar name={c.author ?? "?"} className="mt-0.5 size-6 text-[11px]" />
                  <div className="min-w-0 flex-1 rounded-lg border border-border bg-muted/40 px-3 py-2">
                    <p className="text-[12px]">
                      <span className="font-semibold">{c.author ?? "someone"}</span>{" "}
                      <span className="font-mono text-[10.5px] text-muted-foreground">{fmtAgo(c.at, now)}</span>
                    </p>
                    <Markdown className="mt-0.5 !text-[13.5px]">{c.body}</Markdown>
                  </div>
                </li>
              ))}
            </ul>
            <CommentForm taskId={t.id} />
          </section>

          <p className="border-t border-border pt-3 font-mono text-[10.5px] text-muted-foreground">
            created {fmtAgo(t.created_at, now)} · updated {fmtAgo(t.updated_at, now)} · epoch {t.lease_epoch}
          </p>
        </div>
      )}

      <AlertDialog open={confirmCancel} onOpenChange={setConfirmCancel}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Cancel {t.id}?</AlertDialogTitle>
            <AlertDialogDescription>
              The portion leaves the pot for good: “{t.title}”.{" "}
              {t.status === "claimed" ? "The cook holding it loses its lease and must stop." : "Nobody will scoop it."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void run("cancel", () => call("cancelTask", { id: t.id }), `${t.id} cancelled`)}
            >
              Cancel the task
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <ServeDialog
        t={t}
        mine={mine}
        open={serving}
        onOpenChange={setServing}
        serve={async (result) => {
          let epoch = t.lease_epoch;
          if (!mine) {
            const claim = await call("claimTask", { id: t.id });
            if (!claim.lease) throw new Error("Someone else scooped it first");
            epoch = claim.lease.epoch;
          }
          await call("complete", { id: t.id, epoch, result });
        }}
      />
    </div>
  );
}

function LinkList({
  title,
  items,
  open,
  bare,
}: {
  title: string;
  items: { id: string; title: string; status: string }[];
  open: (id: string) => void;
  bare?: boolean;
}) {
  return (
    <div>
      <SectionLabel>{title}</SectionLabel>
      <ul className="mt-1.5 space-y-1">
        {items.map((l) => (
          <li key={l.id}>
            <button
              type="button"
              onClick={() => open(l.id)}
              className="flex w-full items-center gap-2.5 rounded-lg border border-border bg-card px-3 py-1.5 text-left hover:border-foreground/25"
            >
              <span className={cn("size-2 rounded-full", BUCKET_DOT[bucketOf({ status: l.status, ready: false, lease_live: false })])} aria-hidden="true" />
              <IdChip id={l.id} />
              {!bare && <span className="min-w-0 flex-1 truncate text-[13px]">{l.title}</span>}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function CommentForm({ taskId }: { taskId: string }) {
  const call = useProcCall();
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e?: FormEvent) {
    e?.preventDefault();
    if (!body.trim()) return;
    setBusy(true);
    try {
      await call("comment", { id: taskId, body: body.trim() });
      setBody("");
    } catch (err) {
      toast.error("Could not add the comment", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="mt-3 space-y-2">
      <Label htmlFor="comment" className="sr-only">
        Add a comment
      </Label>
      <Textarea
        id="comment"
        value={body}
        rows={2}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
        }}
        placeholder="Add a note or handoff…"
      />
      <div className="flex items-center justify-between">
        <span className="font-mono text-[10.5px] text-muted-foreground">markdown ok · ⌘↵ to send</span>
        <Button type="submit" size="sm" variant="outline" disabled={busy || !body.trim()}>
          {busy ? <Loader2 className="animate-spin" /> : <Send />} Comment
        </Button>
      </div>
    </form>
  );
}

function EditForm({
  t,
  save,
  onDone,
}: {
  t: TaskDetail;
  save: (a: { title: string; body: string; priority: number; labels: string }) => Promise<unknown>;
  onDone: () => void;
}) {
  const [title, setTitle] = useState(t.title);
  const [body, setBody] = useState(t.body);
  const [labels, setLabels] = useState<string[]>(t.labels);
  const [priority, setPriority] = useState(String(Math.max(-1, Math.min(2, t.priority))));
  const [busy, setBusy] = useState(false);
  const dirty = useMemo(() => title !== t.title || body !== t.body, [title, body, t.title, t.body]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await save({ title: title.trim(), body, priority: Number(priority), labels: labels.join(",") });
      toast.success(`${t.id} updated`);
      onDone();
    } catch (err) {
      toast.error("Could not save", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex-1 space-y-4 px-6 py-5">
      <div className="space-y-1.5">
        <Label htmlFor="e-title">Title</Label>
        <Input id="e-title" required value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="e-body">Description (markdown)</Label>
        <Textarea id="e-body" rows={9} value={body} onChange={(e) => setBody(e.target.value)} />
      </div>
      <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
        <div className="space-y-1.5">
          <Label>Labels</Label>
          <TagInput label="Labels" value={labels} onChange={setLabels} normalize={(s) => s.toLowerCase()} placeholder="add a label" />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="e-prio">Priority</Label>
          <Select value={priority} onValueChange={setPriority}>
            <SelectTrigger id="e-prio" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PRIORITIES.map((p) => (
                <SelectItem key={p.v} value={String(p.v)}>
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onDone}>
          {dirty ? "Discard" : "Close"}
        </Button>
        <Button type="submit" disabled={busy || !title.trim()}>
          {busy && <Loader2 className="animate-spin" />} Save
        </Button>
      </div>
    </form>
  );
}

function ServeDialog({
  t,
  mine,
  open,
  onOpenChange,
  serve,
}: {
  t: TaskDetail;
  mine: boolean;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  serve: (result: string) => Promise<void>;
}) {
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Serve {t.id}</DialogTitle>
          <DialogDescription>
            {mine
              ? "You hold the lease. Mark the portion served."
              : "You scoop it yourself, then serve it in one go. The lease and epoch are yours for that moment."}
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-3"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            try {
              await serve(result.trim() || "Served from the console");
              toast.success(`${t.id} served`);
              onOpenChange(false);
              setResult("");
            } catch (err) {
              toast.error("Could not serve it", { description: errorMessage(err) });
            } finally {
              setBusy(false);
            }
          }}
        >
          {t.open_items > 0 && (
            <p className="rounded-lg bg-corn/15 px-3 py-2 text-[12.5px]">
              {t.open_items} checklist item{t.open_items === 1 ? " is" : "s are"} still open.
            </p>
          )}
          <Label htmlFor="serve-result">What was served? (shown on the task)</Label>
          <Textarea id="serve-result" rows={3} value={result} onChange={(e) => setResult(e.target.value)} placeholder="Merged in #123, deployed to staging." autoFocus />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Not yet
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="animate-spin" /> : <CheckCheck />} Serve it
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
