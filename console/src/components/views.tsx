import { useEffect, useState, type FormEvent } from "react";
import type { TaskView, ViewOut } from "@corntchap/api";
import { ChevronRight, Loader2, Play } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall } from "@/lib/data";
import { client, errorMessage } from "@/lib/api";
import { plural } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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

export type ViewLayout = "list" | "board";

export const layoutOf = (v: ViewOut): ViewLayout => (v.layout === "list" ? "list" : "board");

/** A pot's saved views, in `position` order. */
export function useViews(potKey: string, enabled: boolean) {
  const q = useProc("viewList", { project: potKey }, { enabled });
  const views = [...(q.data ?? [])].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name));
  return { views, isLoading: q.isLoading, error: q.error };
}

const COLUMNS: [string, string][] = [
  ["id", "text, like PAY-12"],
  ["number", "int"],
  ["title", "text"],
  ["body", "text"],
  ["status", "open, claimed, blocked, done, cancelled"],
  ["priority", "int"],
  ["labels", "text[], use 'urgent' = any(labels)"],
  ["menu", "text, the menu key"],
  ["claimed_by", "text, the cook"],
  ["created_by", "text"],
  ["created_at", "timestamptz"],
  ["updated_at", "timestamptz"],
  ["ready", "bool"],
  ["lease_live", "bool"],
  ["waiting_on_question", "bool"],
  ["open_items", "int, unchecked checklist items"],
  ["total_items", "int"],
  ["parent", "text, the parent task id"],
];

const FUNCTIONS =
  "lower, upper, length, coalesce, nullif, greatest, least, now, date_trunc, age, cardinality, array_length, abs, round, starts_with";

const EXAMPLES: [string, string][] = [
  ["menu = 'mvp' and status <> 'done'", ""],
  ["'urgent' = any(labels)", ""],
  ["claimed_by = 'doug'", ""],
  ["created_at > now() - interval '7 days'", ""],
  ["", "priority desc, created_at"],
];

function SqlHelp({ onUse }: { onUse: (where: string, order: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="rounded-lg border border-border bg-muted/40">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="flex w-full items-center gap-1.5 px-3 py-2 text-left text-[12.5px] font-medium text-muted-foreground hover:text-foreground"
      >
        <ChevronRight className={cn("size-3.5 transition-transform", open && "rotate-90")} aria-hidden="true" /> What can I write?
      </button>
      {open && (
        <div className="space-y-3 border-t border-border px-3 py-3 text-[12.5px]">
          <div>
            <p className="label-caps mb-1">Columns</p>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
              {COLUMNS.map(([c, d]) => (
                <div key={c} className="contents">
                  <dt className="font-mono text-[12px]">{c}</dt>
                  <dd className="text-muted-foreground">{d}</dd>
                </div>
              ))}
            </dl>
          </div>
          <div>
            <p className="label-caps mb-1">Functions</p>
            <p className="font-mono text-[12px] break-words text-muted-foreground">{FUNCTIONS}</p>
          </div>
          <div>
            <p className="label-caps mb-1">Examples (click to use)</p>
            <ul className="space-y-1">
              {EXAMPLES.map(([w, o]) => (
                <li key={w + o}>
                  <button
                    type="button"
                    onClick={() => onUse(w, o)}
                    className="rounded bg-card px-1.5 py-0.5 text-left font-mono text-[12px] break-words hover:bg-accent"
                  >
                    {w ? w : <><span className="font-sans text-muted-foreground">order: </span>{o}</>}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}

type Preview = { tasks: TaskView[] } | { error: string } | null;

/** Create (view = null) or edit a saved view. */
export function ViewDialog({
  potKey,
  open,
  view,
  onClose,
  onSaved,
}: {
  potKey: string;
  open: boolean;
  view: ViewOut | null;
  onClose: () => void;
  onSaved: (v: ViewOut) => void;
}) {
  const call = useProcCall();
  const [name, setName] = useState("");
  const [layout, setLayout] = useState<ViewLayout>("board");
  const [where, setWhere] = useState("");
  const [order, setOrder] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const [preview, setPreview] = useState<Preview>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setName(view?.name ?? "");
      setLayout(view ? layoutOf(view) : "board");
      setWhere(view?.where ?? "");
      setOrder(view?.order ?? "");
      setDescription(view?.description ?? "");
      setPreview(null);
      setSaveError(null);
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const edit = (set: (v: string) => void) => (v: string) => {
    set(v);
    setPreview(null);
    setSaveError(null);
  };

  async function runPreview() {
    setPreviewing(true);
    try {
      const tasks = await client.procedures.viewRun({ project: potKey, where: where.trim() || null, orderBy: order.trim() || null });
      setPreview({ tasks });
    } catch (e) {
      setPreview({ error: errorMessage(e) });
    } finally {
      setPreviewing(false);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    setSaveError(null);
    try {
      const v = await call("viewSave", {
        project: potKey,
        name: name.trim(),
        where: where.trim() || null,
        orderBy: order.trim() || null,
        layout,
        key: view?.key ?? null,
        description: description.trim() || null,
      });
      toast.success(view ? `${v.name} updated` : `${v.name} view saved`);
      onSaved(v);
      onClose();
    } catch (err) {
      setSaveError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{view ? `Edit ${view.name}` : "New view"}</DialogTitle>
          <DialogDescription>A view is a SQL condition over this pot's tasks. Write it yourself, or ask a cook to.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="min-w-0 space-y-4">
          <div className="grid gap-4 sm:grid-cols-[1fr_auto]">
            <div className="space-y-1.5">
              <Label htmlFor="vw-name">Name</Label>
              <Input id="vw-name" autoFocus required value={name} onChange={(e) => setName(e.target.value)} placeholder="Urgent on the fire" />
            </div>
            <div className="space-y-1.5">
              <span id="vw-layout" className="text-sm leading-none font-medium">Layout</span>
              <div role="radiogroup" aria-labelledby="vw-layout" className="flex h-9 items-center gap-1 rounded-lg bg-muted p-1">
                {(["board", "list"] as const).map((l) => (
                  <button
                    key={l}
                    type="button"
                    role="radio"
                    aria-checked={layout === l}
                    onClick={() => setLayout(l)}
                    className={cn(
                      "rounded-md px-3 py-1 text-[13px] font-medium text-muted-foreground capitalize transition-colors hover:text-foreground",
                      layout === l && "bg-card text-foreground shadow-sm",
                    )}
                  >
                    {l}
                  </button>
                ))}
              </div>
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="vw-where">Condition (SQL)</Label>
            <Textarea
              id="vw-where"
              rows={3}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              value={where}
              onChange={(e) => edit(setWhere)(e.target.value)}
              placeholder="menu = 'mvp' and status <> 'done'"
              className="font-mono text-[12.5px]"
            />
            <p className="text-[12px] text-muted-foreground">Leave empty to match every task.</p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="vw-order">Order (optional)</Label>
            <Input
              id="vw-order"
              spellCheck={false}
              autoCapitalize="off"
              value={order}
              onChange={(e) => edit(setOrder)(e.target.value)}
              placeholder="priority desc, created_at"
              className="font-mono text-[12.5px]"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="vw-desc">Description (optional)</Label>
            <Input id="vw-desc" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What this view is for." />
          </div>

          <SqlHelp
            onUse={(w, o) => {
              if (w) setWhere(w);
              if (o) setOrder(o);
              setPreview(null);
            }}
          />

          <div className="space-y-2">
            <Button type="button" variant="outline" size="sm" onClick={() => void runPreview()} disabled={previewing}>
              {previewing ? <Loader2 className="animate-spin" /> : <Play />} Preview
            </Button>
            <div aria-live="polite">
              {preview && "error" in preview && (
                <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 font-mono text-[12.5px] break-words text-destructive">
                  {preview.error}
                </p>
              )}
              {preview && "tasks" in preview && (
                <div className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-[12.5px]">
                  <p className="font-medium">
                    {preview.tasks.length >= 500 ? "500 or more tasks (the first 500 run)" : plural(preview.tasks.length, "task")} match
                    {preview.tasks.length === 1 ? "es" : ""}.
                  </p>
                  {preview.tasks.length > 0 && (
                    <ul className="mt-1.5 space-y-0.5">
                      {preview.tasks.slice(0, 6).map((t) => (
                        <li key={t.id} className="flex min-w-0 gap-2">
                          <span className="shrink-0 font-mono text-[12px] text-muted-foreground">{t.id}</span>
                          <span className="truncate">{t.title}</span>
                        </li>
                      ))}
                      {preview.tasks.length > 6 && <li className="text-muted-foreground">and {preview.tasks.length - 6} more</li>}
                    </ul>
                  )}
                </div>
              )}
            </div>
          </div>

          {saveError && (
            <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 font-mono text-[12.5px] break-words text-destructive">
              {saveError}
            </p>
          )}

          <DialogFooter className="gap-2 pt-1">
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !name.trim()}>
              {busy && <Loader2 className="animate-spin" />} {view ? "Save view" : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function DeleteViewDialog({
  potKey,
  view,
  onClose,
  onDeleted,
}: {
  potKey: string;
  view: ViewOut | null;
  onClose: () => void;
  onDeleted: (v: ViewOut) => void;
}) {
  const call = useProcCall();
  const [busy, setBusy] = useState(false);
  return (
    <AlertDialog open={!!view} onOpenChange={(o) => !o && !busy && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete the {view?.name} view?</AlertDialogTitle>
          <AlertDialogDescription>
            The view goes away for everyone on this pot. No task is touched.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy}
            onClick={async (e) => {
              e.preventDefault();
              if (!view) return;
              setBusy(true);
              try {
                await call("viewDelete", { project: potKey, key: view.key });
                toast.success(`${view.name} deleted`);
                onDeleted(view);
                onClose();
              } catch (err) {
                toast.error("Could not delete the view", { description: errorMessage(err) });
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy && <Loader2 className="animate-spin" />} Delete view
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
