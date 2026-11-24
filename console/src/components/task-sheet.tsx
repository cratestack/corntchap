import { useEffect, useMemo, useState, type FormEvent } from "react";
import type { TaskDetail } from "@corntchap/api";
import { CheckCheck, ChevronLeft, Flame, Loader2, MoreHorizontal, Pencil, Send, Trash2, Undo2, Unlock } from "lucide-react";
import { toast } from "sonner";
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
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
import { useProc, useProcCall, useProjects, useV1Call } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { BUCKETS, bucketOf, fmtAgo, useNow, taskNote } from "@/lib/format";
import { useSession } from "@/lib/session";
import { useLive } from "@/lib/live";
import { useIsPhone } from "@/lib/media";
import { useOpenTask } from "@/lib/nav";
import { Avatar, BUCKET_DOT, IdChip, LabelChip, LeaseTimer, ProgressBar, RepoChip, SectionLabel } from "./bits";
import { RepoPicker } from "./repos";
import { OwnerPicker } from "./owner-picker";
import { TaskPrs } from "./forge-prs";
import { TaskReviews } from "./reviews";
import { RefreshButton } from "./refresh-button";
import { Markdown } from "./markdown";
import { TagInput } from "./tag-input";
import { PRIORITIES } from "./new-task";
import { MenuChip, useMenus } from "./menus";
import { Attachments } from "./files";
import { EventFeed } from "./event-feed";
import { ParkedQuestion, parkedQuestionRef } from "./questions";
import { projectOfTask } from "@/lib/nav";
import { cn } from "@/lib/utils";

export function TaskSheet({ id, onClose }: { id: string | null; onClose: () => void }) {
  const phone = useIsPhone();
  return (
    <Sheet open={!!id} onOpenChange={(o) => !o && onClose()}>
      <SheetContent
        side="right"
        showCloseButton={!phone}
        className="w-full gap-0 overflow-hidden p-0 sm:max-w-[34rem]"
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

/** The sheet's top bar: back (phones), the id, and whatever actions the caller passes. Stays put while the body scrolls. */
function SheetTop({ id, children }: { id: string; children?: React.ReactNode }) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-1 border-b border-border bg-card pr-2 pl-1.5 md:pr-12 md:pl-5">
      <SheetClose asChild>
        <Button variant="ghost" size="icon" aria-label="Back to the board" className="md:hidden">
          <ChevronLeft className="size-5" />
        </Button>
      </SheetClose>
      <IdChip id={id} className="text-[13px]" />
      <div className="ml-auto flex items-center gap-1">{children}</div>
    </div>
  );
}

function Prop({ children, className, title }: { children: React.ReactNode; className?: string; title?: string }) {
  return (
    <span title={title} className={cn("inline-flex h-8 items-center gap-1.5 rounded-full border border-border bg-card px-3 text-[12.5px] whitespace-nowrap", className)}>
      {children}
    </span>
  );
}

function TaskDetailBody({ id }: { id: string }) {
  const q = useProc("taskDetail", { id });
  const me = useSession();
  const call = useProcCall();
  const v1 = useV1Call();
  const open = useOpenTask();
  const now = useNow();
  const { events } = useLive();
  const [editing, setEditing] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [tossReason, setTossReason] = useState("");
  const [serving, setServing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => setEditing(false), [id]);

  const t = q.data;
  const activity = useMemo(() => events.filter((e) => e.task === id && e.kind !== "task.commented"), [events, id]);

  if (q.error && !t) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <SheetTop id={id} />
        <div className="p-6">
          <h2 className="text-2xl">That task is not in the pot</h2>
          <p className="mt-2 text-muted-foreground">{errorMessage(q.error)}</p>
        </div>
      </div>
    );
  }
  if (!t) {
    return (
      <div className="flex min-h-0 flex-1 flex-col" aria-busy="true" aria-label="Loading task">
        <SheetTop id={id} />
        <div className="space-y-5 p-5">
          <Skeleton className="h-8 w-3/4" />
          <div className="flex gap-2">
            <Skeleton className="h-8 w-24 rounded-full" />
            <Skeleton className="h-8 w-20 rounded-full" />
            <Skeleton className="h-8 w-16 rounded-full" />
          </div>
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-28 w-full" />
        </div>
      </div>
    );
  }

  const bucket = bucketOf(t);
  const mine = t.claimed_by === me.name && !!t.lease_live;
  const closed = t.status === "done" || t.status === "cancelled";
  const done = t.total_items - t.open_items;
  const note = taskNote(t as never);
  const parkedRef = parkedQuestionRef(t);
  const canServe = !closed && (bucket === "ready" || mine);
  const prio = t.priority !== 0 ? PRIORITIES.find((p) => p.v === Math.max(-1, Math.min(2, t.priority)))?.label : null;

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

  const restore = () => run("restore", () => call("restoreTask", { id: t.id }), `${t.id} is back on the counter`);
  const unblock = () => run("unblock", () => call("unblockTask", { id: t.id }), `${t.id} is back in the pot`);
  const hasMenuActions = !closed || t.status === "cancelled" || t.status === "blocked";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SheetTop id={t.id}>
        <RefreshButton size="icon-sm" />
        {hasMenuActions && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon" aria-label="Task actions" disabled={!!busy}>
                <MoreHorizontal className="size-5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-44">
              {!closed && (
                <DropdownMenuItem className="min-h-11" onSelect={() => setEditing(true)}>
                  <Pencil /> Edit
                </DropdownMenuItem>
              )}
              {t.status === "blocked" && (
                <DropdownMenuItem className="min-h-11" onSelect={() => void unblock()}>
                  <Unlock /> Unblock
                </DropdownMenuItem>
              )}
              {t.status === "cancelled" && (
                <DropdownMenuItem className="min-h-11" onSelect={() => void restore()}>
                  <Undo2 /> Restore
                </DropdownMenuItem>
              )}
              {!closed && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem variant="destructive" className="min-h-11" onSelect={() => setConfirmCancel(true)}>
                    <Trash2 /> Toss…
                  </DropdownMenuItem>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </SheetTop>

      {editing ? (
        <EditForm t={t} onDone={() => setEditing(false)} save={(args) => call("taskEdit", { id: t.id, ...args })} />
      ) : (
        <>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
            <header className="px-5 pt-4 pb-4 md:px-6 md:pt-5">
              <h2 className="text-[24px] leading-[1.15] md:text-[26px]">{t.title}</h2>
              <div className="mt-3.5 flex flex-wrap items-center gap-2" role="group" aria-label="Properties">
                <Prop>
                  <span className={cn("size-2 rounded-full", BUCKET_DOT[bucket], bucket === "claimed" && "live-dot")} aria-hidden="true" />
                  {BUCKETS.find((b) => b.id === bucket)?.label}
                </Prop>
                {t.claimed_by && (
                  <Prop>
                    <Avatar name={t.claimed_by} className="size-4 rounded-[5px] text-[9px]" /> {t.claimed_by}
                  </Prop>
                )}
                {t.status === "claimed" && (
                  <Prop>
                    <LeaseTimer task={t} />
                  </Prop>
                )}
                {prio && (
                  <Prop className={cn("font-mono text-[11px] uppercase tracking-wider", t.priority > 0 ? "border-primary/40 bg-primary/10 text-primary" : "text-muted-foreground")}>
                    {t.priority >= 2 && <Flame className="size-3" aria-hidden="true" />}
                    {prio}
                  </Prop>
                )}
                <OwnerPicker
                  pot={t.project}
                  value={t.owner ?? null}
                  pill
                  disabled={!!busy}
                  onChange={(owner) => run("change the owner", () => call("taskEdit", { id: t.id, owner }), owner ? `${owner} owns ${t.id}` : "Owner cleared")}
                />
                <MenuPicker
                  taskId={t.id}
                  current={t.menu ?? null}
                  closed={closed}
                  disabled={!!busy}
                  save={(menu) => run("move it to that menu", () => call("taskEdit", { id: t.id, menu }), menu ? "Moved to the menu" : "Taken off the menu")}
                />
                {t.repos.map((r) => (
                  <RepoChip key={r} className="h-8 px-3 text-[12.5px]">
                    {r}
                  </RepoChip>
                ))}
                {t.labels.map((l) => (
                  <LabelChip key={l} className="h-8 px-3 text-[12.5px]">
                    {l}
                  </LabelChip>
                ))}
              </div>
              {note && <p className="mt-3 text-[12.5px] text-muted-foreground">{note}</p>}
              {(canServe || t.status === "blocked" || t.status === "cancelled") && (
                <div className="mt-4 flex gap-2">
                  {canServe && (
                    <Button className="flex-1 md:flex-none" onClick={() => setServing(true)} disabled={!!busy}>
                      <CheckCheck /> Serve
                    </Button>
                  )}
                  {t.status === "blocked" && (
                    <Button className="flex-1 md:flex-none" variant="outline" disabled={!!busy} onClick={() => void unblock()}>
                      {busy === "unblock" ? <Loader2 className="animate-spin" /> : <Unlock />} Unblock
                    </Button>
                  )}
                  {t.status === "cancelled" && (
                    <Button className="flex-1 md:flex-none" variant="outline" disabled={!!busy} onClick={() => void restore()}>
                      {busy === "restore" ? <Loader2 className="animate-spin" /> : <Undo2 />} Restore
                    </Button>
                  )}
                </div>
              )}
            </header>

            <div className="space-y-7 border-t border-border px-5 py-5 md:px-6">
              {parkedRef && <ParkedQuestion id={parkedRef} pot={projectOfTask(t.id)} />}
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

              <TaskPrs task={t} />

              <TaskReviews task={t} />

              <section aria-labelledby="ck">
                <div className="mb-2 flex items-center justify-between">
                  <SectionLabel>
                    <span id="ck">Checklist</span>
                  </SectionLabel>
                  {t.total_items > 0 && <ProgressBar done={done} total={t.total_items} />}
                </div>
                {t.items.length === 0 ? (
                  <p className="text-muted-foreground">No checklist items.</p>
                ) : (
                  <ul className="-mx-2 divide-y divide-border/60">
                    {t.items.map((it) => (
                      <li key={it.position}>
                        <label className="flex min-h-12 cursor-pointer items-center gap-3.5 rounded-lg px-2 py-2 hover:bg-muted/60 active:bg-muted/60 md:min-h-10 md:gap-3">
                          <Checkbox
                            checked={it.done}
                            disabled={!!busy || t.status === "cancelled"}
                            onCheckedChange={(c) => void tick(it.position, c === true)}
                            aria-label={`${it.done ? "Untick" : "Tick"}: ${it.text}`}
                            className="size-6 rounded-md md:size-4 md:rounded-[4px] [&_svg]:size-4 md:[&_svg]:size-3.5"
                          />
                          <span className="min-w-0 flex-1">
                            <span className={cn("text-[14.5px] leading-snug md:text-[13.5px]", it.done && "text-muted-foreground line-through decoration-muted-foreground/50")}>{it.text}</span>
                            {it.done && it.done_by && <span className="ml-2 font-mono text-[10.5px] text-muted-foreground">{it.done_by}</span>}
                          </span>
                          <span className="font-mono text-[10.5px] text-muted-foreground/70">{it.position}</span>
                        </label>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              <Attachments target={{ pot: projectOfTask(t.id), task: t.id }} files={t.files} />

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
                <ul className="mt-3 space-y-3">
                  {t.comments.length === 0 && <li className="text-muted-foreground">No comments yet. Leave a note for the next cook.</li>}
                  {t.comments.map((c, i) => (
                    <li key={i} className="flex gap-3">
                      <Avatar name={c.author ?? "?"} className="mt-0.5 size-7 text-[12px]" />
                      <div className="min-w-0 flex-1 rounded-xl border border-border bg-muted/40 px-3.5 py-2.5">
                        <p className="text-[12px]">
                          <span className="font-semibold">{c.author ?? "someone"}</span>{" "}
                          {c.station_label && (
                            <span className="font-mono text-[10.5px] text-muted-foreground" title={`Session ${c.station}`}>
                              · {c.station_label}{" "}
                            </span>
                          )}
                          <span className="font-mono text-[10.5px] text-muted-foreground">{fmtAgo(c.at, now)}</span>
                        </p>
                        <Markdown className="mt-0.5 !text-[14px]">{c.body}</Markdown>
                      </div>
                    </li>
                  ))}
                </ul>
              </section>

              {activity.length > 0 && (
                <section aria-labelledby="ac">
                  <SectionLabel>
                    <span id="ac">Activity</span>
                  </SectionLabel>
                  <EventFeed events={activity} compact limit={8} />
                </section>
              )}

              <p className="border-t border-border pt-3 font-mono text-[10.5px] text-muted-foreground">
                created {fmtAgo(t.created_at, now)} · updated {fmtAgo(t.updated_at, now)} · epoch {t.lease_epoch}
              </p>
            </div>
          </div>
          <CommentForm taskId={t.id} />
        </>
      )}

      <AlertDialog open={confirmCancel} onOpenChange={setConfirmCancel}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Toss {t.id}?</AlertDialogTitle>
            <AlertDialogDescription>
              “{t.title}” goes to the Tossed shelf: nobody will scoop it.{" "}
              {t.status === "claimed" ? "The cook holding it loses its lease and must stop. " : ""}
              You can restore it later.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <Textarea
            aria-label="Why is it tossed? (optional)"
            placeholder="Why? (optional, shown on the task) e.g. duplicate of PAY-12"
            rows={2}
            value={tossReason}
            onChange={(e) => setTossReason(e.target.value)}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() =>
                void run(
                  "cancel",
                  () => call("cancelTask", { id: t.id, reason: tossReason.trim() || null }),
                  `${t.id} tossed`,
                ).then(() => setTossReason(""))
              }
            >
              Toss it
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

function MenuPicker({
  taskId,
  current,
  closed,
  disabled,
  save,
}: {
  taskId: string;
  current: string | null;
  closed: boolean;
  disabled: boolean;
  save: (menu: string) => void;
}) {
  const menus = useMenus(projectOfTask(taskId));
  const name = current ? (menus.names[current] ?? current) : null;
  if (closed || (menus.open.length === 0 && !current)) {
    return name ? <MenuChip name={name} className="h-8 px-3 text-[12.5px]" /> : null;
  }
  const options = [...menus.open, ...menus.menus.filter((m) => m.key === current && m.status === "served")];
  return (
    <Select value={current ?? "-"} onValueChange={(v) => save(v === "-" ? "" : v)} disabled={disabled}>
      <SelectTrigger aria-label="Menu" size="sm" className="h-8 min-w-0 max-w-48 gap-1.5 rounded-full border-border px-3 text-[12.5px] data-[size=sm]:h-8">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="-">No menu</SelectItem>
        {options.map((m) => (
          <SelectItem key={m.key} value={m.key}>
            {m.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
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
              className="flex min-h-12 w-full items-center gap-2.5 rounded-lg border border-border bg-card px-3 py-1.5 text-left hover:border-foreground/25 md:min-h-0"
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
    <form
      onSubmit={submit}
      className="flex shrink-0 items-end gap-2 border-t border-border bg-card px-3 py-2.5 pb-[calc(0.625rem+env(safe-area-inset-bottom))] md:px-5 md:pb-3"
    >
      <Label htmlFor="comment" className="sr-only">
        Add a comment
      </Label>
      <Textarea
        id="comment"
        value={body}
        rows={1}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
        }}
        placeholder="Add a note or handoff…"
        className="max-h-36 min-h-11 flex-1 resize-none rounded-xl py-2.5"
      />
      <Button type="submit" size="icon" aria-label="Send comment" title="Send (Ctrl or Cmd + Enter)" disabled={busy || !body.trim()}>
        {busy ? <Loader2 className="animate-spin" /> : <Send />}
      </Button>
    </form>
  );
}

function EditForm({
  t,
  save,
  onDone,
}: {
  t: TaskDetail;
  save: (a: { title: string; body: string; priority: number; labels: string; repos?: string }) => Promise<unknown>;
  onDone: () => void;
}) {
  const [title, setTitle] = useState(t.title);
  const [body, setBody] = useState(t.body);
  const [labels, setLabels] = useState<string[]>(t.labels);
  const potRepos = useProjects().data?.find((p) => p.key === t.project)?.repos ?? [];
  const [repos, setRepos] = useState<string[]>(t.repos);
  const [priority, setPriority] = useState(String(Math.max(-1, Math.min(2, t.priority))));
  const [busy, setBusy] = useState(false);
  const dirty = useMemo(() => title !== t.title || body !== t.body, [title, body, t.title, t.body]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await save({ title: title.trim(), body, priority: Number(priority), labels: labels.join(","), ...(potRepos.length > 0 ? { repos: repos.join(",") } : {}) });
      toast.success(`${t.id} updated`);
      onDone();
    } catch (err) {
      toast.error("Could not save", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-5 md:px-6">
      <div className="space-y-1.5">
        <Label htmlFor="e-title">Title</Label>
        <Input id="e-title" required value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="e-body">Description (markdown)</Label>
        <Textarea id="e-body" rows={9} value={body} onChange={(e) => setBody(e.target.value)} />
      </div>
      {potRepos.length > 0 && (
        <div className="space-y-1.5">
          <Label>Repositories</Label>
          <RepoPicker repos={potRepos} value={repos} onChange={setRepos} label="Repositories the task is about" />
          <p className="text-[12px] text-muted-foreground">None chosen: any repository.</p>
        </div>
      )}
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
      </div>
      <div className="flex shrink-0 justify-end gap-2 border-t border-border bg-card px-5 py-3 pb-[calc(0.75rem+env(safe-area-inset-bottom))] md:px-6">
        <Button type="button" variant="ghost" onClick={onDone} className="max-md:flex-1">
          {dirty ? "Discard" : "Close"}
        </Button>
        <Button type="submit" disabled={busy || !title.trim()} className="max-md:flex-1">
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
