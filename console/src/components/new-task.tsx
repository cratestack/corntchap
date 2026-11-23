import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useNavigate } from "react-router";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useUi } from "@/lib/ui";
import { useProc, useProcCall, useProjects } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { lastView } from "@/lib/views";
import { TagInput } from "./tag-input";
import { RepoPicker } from "./repos";
import { useMenus } from "./menus";

export const PRIORITIES = [
  { v: -1, label: "Low" },
  { v: 0, label: "Normal" },
  { v: 1, label: "High" },
  { v: 2, label: "Urgent" },
];

export function NewTaskDialog() {
  const { newTask, closeNewTask } = useUi();
  const projects = useProjects();
  const call = useProcCall();
  const navigate = useNavigate();

  const [pot, setPot] = useState("");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [items, setItems] = useState("");
  const [labels, setLabels] = useState<string[]>([]);
  const [deps, setDeps] = useState<string[]>([]);
  const [priority, setPriority] = useState("0");
  const [menu, setMenu] = useState("-");
  const [repos, setRepos] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (newTask.open) {
      setPot(newTask.pot ?? projects.data?.[0]?.key ?? "");
      setTitle("");
      setBody("");
      setItems("");
      setLabels([]);
      setDeps([]);
      setRepos([]);
      setPriority("0");
      setMenu(newTask.menu && newTask.menu !== "-" ? newTask.menu : "-");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [newTask.open]);

  useEffect(() => {
    // functional: on open this runs in the same commit as the reset above, so `pot` here is still the old ""
    const first = projects.data?.[0]?.key;
    if (newTask.open && !pot && first) setPot((p) => p || first);
  }, [newTask.open, pot, projects.data]);

  const potRepos = projects.data?.find((p) => p.key === pot)?.repos ?? [];
  const menus = useMenus(pot, newTask.open);
  const tasks = useProc("projectTasks", { project: pot }, { enabled: newTask.open && !!pot });
  const depSuggestions = useMemo(
    () => (tasks.data ?? []).filter((t) => t.status !== "cancelled").map((t) => t.id),
    [tasks.data],
  );

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!title.trim() || !pot) return;
    setBusy(true);
    try {
      const t = await call("taskCreate", {
        project: pot,
        title: title.trim(),
        body: body.trim() || null,
        items: items.trim() || null,
        labels: labels.length ? labels.join(",") : null,
        dependsOn: deps.length ? deps.join(",") : null,
        priority: Number(priority),
        repos: potRepos.length > 0 && repos.length > 0 ? repos.join(",") : null,
        menu: menu !== "-" && menus.open.some((m) => m.key === menu) ? menu : null,
      });
      toast.success(`${t.id} is in the pot`, {
        description: t.title,
        action: { label: "Open", onClick: () => void navigate(`/pots/${pot}/${lastView(pot)}?task=${t.id}`) },
      });
      closeNewTask();
    } catch (err) {
      toast.error("Could not add the task", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  const noPots = projects.data && projects.data.length === 0;

  return (
    <Dialog open={newTask.open} onOpenChange={(o) => !o && closeNewTask()}>
      <DialogContent className="max-h-[92dvh] overflow-hidden sm:max-w-xl max-sm:flex max-sm:flex-col max-sm:gap-0 max-sm:p-0 sm:flex sm:flex-col sm:gap-0 sm:p-0">
        <DialogHeader className="px-5 pt-5 pb-3 text-left max-sm:pt-7 sm:px-6 sm:pt-6">
          <DialogTitle>New task</DialogTitle>
          <DialogDescription className="max-sm:sr-only">Add a portion to the pot. A cook scoops it when nothing is in the way.</DialogDescription>
        </DialogHeader>
        {noPots ? (
          <p className="px-5 py-4 text-muted-foreground">There is no pot to put it in yet. An admin can start one under Pots.</p>
        ) : (
          <form onSubmit={submit} className="flex min-h-0 flex-1 flex-col">
            <div className="min-h-0 flex-1 space-y-4 overflow-y-auto border-t border-border px-5 py-4 sm:px-6">
              <div className="space-y-1.5">
                <Label htmlFor="nt-title">Title</Label>
                <Input id="nt-title" autoFocus required value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Retry failed webhook deliveries with backoff" />
              </div>
              <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,8.5rem)] gap-3 sm:grid-cols-[1fr_9rem] sm:gap-4">
                <div className="space-y-1.5">
                  <Label htmlFor="nt-pot">Pot</Label>
                  <Select
                    value={pot}
                    // Radix reports "" while its options mount: keep the pot the dialog was opened for
                    onValueChange={(v) => {
                      if (!v || v === pot) return;
                      setPot(v);
                      setMenu("-");
                      setRepos([]);
                    }}
                  >
                    <SelectTrigger id="nt-pot" className="w-full">
                      <SelectValue placeholder="Choose a pot" />
                    </SelectTrigger>
                    <SelectContent>
                      {(projects.data ?? []).map((p) => (
                        <SelectItem key={p.key} value={p.key}>
                          <span className="font-mono text-[12px] font-bold">{p.key}</span> {p.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="nt-prio">Priority</Label>
                  <Select value={priority} onValueChange={setPriority}>
                    <SelectTrigger id="nt-prio" className="w-full">
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
              {menus.open.length > 0 && (
                <div className="space-y-1.5">
                  <Label htmlFor="nt-menu">Menu</Label>
                  <Select value={menu} onValueChange={setMenu}>
                    <SelectTrigger id="nt-menu" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="-">No menu</SelectItem>
                      {menus.open.map((m) => (
                        <SelectItem key={m.key} value={m.key}>
                          {m.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}
              {potRepos.length > 0 && (
                <div className="space-y-1.5">
                  <Label>Repositories</Label>
                  <RepoPicker repos={potRepos} value={repos} onChange={setRepos} label="Repositories the task is about" />
                  <p className="text-[12px] text-muted-foreground">None chosen: any repository. A cook that handles other repositories will not scoop it.</p>
                </div>
              )}
              <div className="space-y-1.5">
                <Label htmlFor="nt-body">Description (markdown)</Label>
                <Textarea id="nt-body" rows={4} value={body} onChange={(e) => setBody(e.target.value)} placeholder="What done looks like, files to look at, gotchas." />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="nt-items">Checklist, one item per line</Label>
                <Textarea id="nt-items" rows={3} value={items} onChange={(e) => setItems(e.target.value)} className="font-mono text-[12.5px]" placeholder={"Write the failing test\nImplement backoff\nUpdate the runbook"} />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <Label>Labels</Label>
                  <TagInput label="Labels" value={labels} onChange={setLabels} placeholder="backend, urgent" normalize={(s) => s.toLowerCase()} />
                </div>
                <div className="space-y-1.5">
                  <Label>Depends on</Label>
                  <TagInput label="Depends on" value={deps} onChange={setDeps} placeholder={`${pot || "PAY"}-1`} suggestions={depSuggestions} normalize={(s) => s.toUpperCase()} />
                </div>
              </div>
            </div>
            <DialogFooter className="shrink-0 flex-row justify-end gap-2 border-t border-border bg-card px-5 py-3 pb-[calc(0.75rem+env(safe-area-inset-bottom))] sm:px-6 sm:pb-3">
              <Button type="button" variant="ghost" onClick={closeNewTask} className="max-sm:flex-1">
                Cancel
              </Button>
              <Button type="submit" disabled={busy || !title.trim() || !pot} className="max-sm:flex-[2]">
                {busy && <Loader2 className="animate-spin" />} Add to the pot
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
