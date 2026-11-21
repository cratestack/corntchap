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
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (newTask.open) {
      setPot(newTask.pot ?? projects.data?.[0]?.key ?? "");
      setTitle("");
      setBody("");
      setItems("");
      setLabels([]);
      setDeps([]);
      setPriority("0");
      setMenu(newTask.menu && newTask.menu !== "-" ? newTask.menu : "-");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [newTask.open]);

  useEffect(() => {
    if (newTask.open && !pot && projects.data?.[0]) setPot(projects.data[0].key);
  }, [newTask.open, pot, projects.data]);

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
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>New task</DialogTitle>
          <DialogDescription>Add a portion to the pot. A cook scoops it when nothing is in the way.</DialogDescription>
        </DialogHeader>
        {noPots ? (
          <p className="py-4 text-muted-foreground">There is no pot to put it in yet. An admin can start one under Pots.</p>
        ) : (
          <form onSubmit={submit} className="space-y-4">
            <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
              <div className="space-y-1.5">
                <Label htmlFor="nt-pot">Pot</Label>
                <Select value={pot} onValueChange={(v) => { setPot(v); setMenu("-"); }}>
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
            <div className="space-y-1.5">
              <Label htmlFor="nt-title">Title</Label>
              <Input id="nt-title" autoFocus required value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Retry failed webhook deliveries with backoff" />
            </div>
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
            <DialogFooter className="gap-2 pt-1">
              <Button type="button" variant="ghost" onClick={closeNewTask}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy || !title.trim() || !pot}>
                {busy && <Loader2 className="animate-spin" />} Add to the pot
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
