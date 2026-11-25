import { useState } from "react";
import { Link } from "react-router";
import type { ActivityView, StationView } from "@corntchap/api";
import { Activity, AppWindow, ChevronDown, Hourglass, Loader2, Play, SquareTerminal, Star, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useProc, useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
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
import { cn } from "@/lib/utils";
import { IdChip, LabelChip, RepoChip } from "./bits";
import { StationLink } from "./station-link";

const RUNTIME: Record<string, string> = {
  opencode: "OpenCode",
  "claude-code": "Claude Code",
  codex: "Codex",
  copilot: "Copilot",
  antigravity: "Antigravity",
  other: "Other",
  "claude-routine": "Claude routine",
  devin: "Devin",
  coder: "Coder",
  webhook: "Webhook",
  "github-comment": "GitHub comment",
};

const shortSession = (s: string) => (s.length > 14 ? `${s.slice(0, 12)}…` : s);

function TaskButton({ id, children }: { id: string; children?: React.ReactNode }) {
  const open = useOpenTask();
  return (
    <button
      type="button"
      onClick={() => open(id)}
      className="inline-flex items-center gap-1 rounded-md border border-border bg-background px-1.5 py-px hover:border-foreground/40 focus-visible:border-foreground/40"
    >
      {children}
      <IdChip id={id} className="text-[11px]" />
    </button>
  );
}

const activityLine = (a: ActivityView) => {
  const what = a.files.length > 0 ? `${a.files[0]}${a.files.length > 1 ? ` +${a.files.length - 1}` : ""}` : (a.summary ?? "");
  return `${a.tool}${what ? ` ${what}` : ""}`;
};

/** What the station's agent last did (from its hooks): a one-line summary, expandable to the recent list with checkpoints. */
function StationActivity({ station, now }: { station: StationView; now: number }) {
  const [open, setOpen] = useState(false);
  const { data } = useProc("activityList", { project: station.project, station: station.id, limit: 15 }, { refetchMs: 15000 });
  const list = data ?? [];
  const last = list[0];
  if (!last) return null;
  return (
    <div className="mt-1.5 text-[11.5px]">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 rounded-md text-left text-muted-foreground hover:text-foreground focus-visible:text-foreground"
      >
        <Activity className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono" title={last.summary ?? activityLine(last)}>
          {activityLine(last)}
        </span>
        <span className="shrink-0 font-mono text-[10.5px]">{fmtAgo(last.at, now)}</span>
        <ChevronDown className={cn("size-3.5 shrink-0 transition-transform", open && "rotate-180")} aria-hidden="true" />
      </button>
      {open && (
        <ul className="mt-1 space-y-1 border-l border-border pl-2.5">
          {list.map((a) => (
            <li key={a.seq} className="flex items-baseline gap-2">
              <span className="min-w-0 flex-1 truncate font-mono" title={a.files.length > 0 ? a.files.join("\n") : (a.summary ?? "")}>
                {activityLine(a)}
                {a.count > 1 && <span className="text-muted-foreground"> x{a.count}</span>}
              </span>
              {a.checkpoint && (
                <code className="shrink-0 rounded bg-muted px-1 font-mono text-[10.5px]" title={`Checkpoint ${a.checkpoint}: restore a file with git restore --source ${a.checkpoint} -- <path>`}>
                  {a.checkpoint.slice(0, 7)}
                </code>
              )}
              <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{fmtAgo(a.at, now)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** A cook's registered agent sessions. Registration happens in the CLI; the console shows and removes them. */
export function StationList({ stations, cook, kind }: { stations: StationView[]; cook?: string; kind?: string | null }) {
  const call = useProcCall();
  const now = useNow();
  const [removing, setRemoving] = useState<StationView | null>(null);
  const [busy, setBusy] = useState(false);
  const [starting, setStarting] = useState(false);

  async function copyOpen(s: StationView) {
    try {
      await navigator.clipboard.writeText(`corntchap-cli station open ${s.id}`);
      toast.success("Command copied", {
        description: "Run it on that machine: it opens the session in Claude Desktop (claude --desktop --resume). Add --terminal to resume it in a terminal instead.",
      });
    } catch {
      toast.error("Could not copy the command", { description: `Run corntchap-cli station open ${s.id} on that machine.` });
    }
  }

  async function setMain(s: StationView, main: boolean) {
    try {
      await call("stationSetMain", { id: s.id, main });
      toast.success(main ? "Main station set" : "No main station", {
        description: main
          ? `${s.label ?? s.session} now answers ${s.cook}'s messages and mentions in ${s.project}.`
          : `The session woken last answers ${s.cook}'s messages in ${s.project}.`,
      });
    } catch (e) {
      toast.error("Could not change the main station", { description: errorMessage(e) });
    }
  }

  async function remove(s: StationView) {
    setBusy(true);
    try {
      await call("stationRemove", { id: s.id });
      toast.success("Station removed", { description: "That session will no longer be woken." });
      setRemoving(null);
    } catch (e) {
      toast.error("Could not remove it", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  const canStart = kind === "claude-code" && !!cook;
  const startable = stations.filter((s) => !!s.directory);

  return (
    <div className="space-y-3">
      {canStart && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" className="max-sm:w-full" disabled={startable.length === 0} onClick={() => setStarting(true)}>
            <Play /> Start a session
          </Button>
          {startable.length === 0 && (
            <span className="text-[12px] text-muted-foreground">
              Register a session first: run <code className="font-mono text-[11.5px]">corntchap-cli station start</code> or <code className="font-mono text-[11.5px]">station scan</code> in the repo.
            </span>
          )}
        </div>
      )}
      {canStart && <StartSessionDialog open={starting} onOpenChange={setStarting} cook={cook} stations={startable} />}
      {stations.length === 0 ? (
        <p className="text-[12.5px] text-muted-foreground">
          No stations yet. Run <code className="font-mono text-[11.5px]">corntchap-cli station scan</code> in a repo to register this cook’s sessions.
        </p>
      ) : (
        <>
          <p className="text-[12px] text-muted-foreground">
            The fire wakes each station’s session directly, so work resumes where it left off. The main station <Star className="inline size-3 align-[-1px]" aria-hidden="true" /> answers messages and mentions in its pot.
          </p>
          <ul className="space-y-2">
            {stations.map((s) => {
              const where = s.label ?? s.directory ?? s.session;
              return (
                <li key={s.id} className={cn("rounded-xl border bg-muted/40 px-3.5 py-3 text-[12.5px]", s.main ? "border-primary/50" : "border-border")}>
                  <div className="flex items-start gap-2">
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-[12px] font-medium" title={s.directory ?? where}>
                        {where}
                      </p>
                      {s.label && s.directory && (
                        <p className="truncate font-mono text-[11px] text-muted-foreground" title={s.directory}>
                          {s.directory}
                        </p>
                      )}
                    </div>
                    {s.runtime === "claude-code" && s.directory && (
                      <Button
                        size="icon-sm"
                        variant="ghost"
                        aria-label={`Open ${where} in Claude Desktop`}
                        title="Open in Claude Desktop"
                        onClick={() => void copyOpen(s)}
                      >
                        <AppWindow />
                      </Button>
                    )}
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-pressed={s.main}
                      aria-label={s.main ? `${where} is the main station of ${s.project}: unset it` : `Make ${where} the main station of ${s.project}`}
                      title={s.main ? "Main station: answers messages and mentions (click to unset)" : "Make it the main station: it answers messages and mentions"}
                      onClick={() => void setMain(s, !s.main)}
                    >
                      <Star className={cn(s.main && "fill-primary text-primary")} />
                    </Button>
                    <Button size="icon-sm" variant="ghost" aria-label={`Remove station ${where}`} onClick={() => setRemoving(s)}>
                      <Trash2 />
                    </Button>
                  </div>
                  <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
                    <span className="inline-flex items-center gap-1 text-muted-foreground">
                      <SquareTerminal className="size-3.5" aria-hidden="true" /> {RUNTIME[s.runtime] ?? s.runtime}
                    </span>
                    <LabelChip className="font-mono"><Link to={`/pots/${s.project}/about`} className="hover:underline focus-visible:underline">{s.project}</Link></LabelChip>
                    {s.repo && <RepoChip>{s.repo}</RepoChip>}
                    {s.main && <span className="label-caps text-primary">main</span>}
                    <span className="font-mono text-[11px] text-muted-foreground" title={`Session ${s.session}`}>
                      {shortSession(s.session)}
                    </span>
                  </div>
                  {(s.holding.length > 0 || s.waiting.length > 0) && (
                    <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                      {s.holding.length > 0 && <span className="text-[11.5px] text-muted-foreground">holding</span>}
                      {s.holding.map((t) => (
                        <TaskButton key={t} id={t} />
                      ))}
                      {s.waiting.length > 0 && <span className="text-[11.5px] text-muted-foreground">waiting on a question for</span>}
                      {s.waiting.map((t) => (
                        <TaskButton key={t} id={t}>
                          <Hourglass className="size-3 text-corn-text" aria-hidden="true" />
                        </TaskButton>
                      ))}
                    </div>
                  )}
                  <StationActivity station={s} now={now} />
                  <p className="mt-1.5 font-mono text-[10.5px] text-muted-foreground">
                    {s.last_woken_at ? `woken ${fmtAgo(s.last_woken_at, now)}` : "never woken yet"}
                  </p>
                </li>
              );
            })}
          </ul>
        </>
      )}
      <AlertDialog open={!!removing} onOpenChange={(o) => !o && !busy && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this station?</AlertDialogTitle>
            <AlertDialogDescription>
              The fire stops waking <b>{removing?.label ?? removing?.directory ?? removing?.session}</b>
              {removing && removing.holding.length + removing.waiting.length > 0 ? "; the tasks it holds stay as they are" : ""}. Scan again to register it back.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(e) => {
                e.preventDefault();
                if (removing) void remove(removing);
              }}
            >
              {busy && <Loader2 className="animate-spin" />} Remove it
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function StartSessionDialog({
  open,
  onOpenChange,
  cook,
  stations,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  cook: string;
  stations: StationView[];
}) {
  const call = useProcCall();
  const pots = [...new Set(stations.map((s) => s.project))];
  const [pot, setPot] = useState(pots[0] ?? "");
  const [directory, setDirectory] = useState("");
  const [task, setTask] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);

  const activePot = pots.includes(pot) ? pot : (pots[0] ?? "");
  const inPot = stations.filter((s) => s.project === activePot);
  const dirs = [...new Set([...inPot.filter((s) => s.main), ...inPot].map((s) => s.directory!))];
  const activeDir = dirs.includes(directory) ? directory : (dirs[0] ?? "");

  const t = task.trim();
  const p = prompt.trim();
  const text = p || (t ? `Take task ${t} in corntchap pot ${activePot}` : `Take the next ready task in corntchap pot ${activePot}`);
  const desktop = `claude://code/new?folder=${encodeURIComponent(activeDir)}&q=${encodeURIComponent(text)}`;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const s = await call("stationStart", {
        project: activePot,
        cook,
        task: t || null,
        prompt: p || null,
        directory: activeDir,
      });
      toast.success("Session started", {
        description: `${s.label ?? s.session} starts on ${cook}’s machine when its relay (corntchap-cli fire) picks it up.`,
      });
      setTask("");
      setPrompt("");
      onOpenChange(false);
    } catch (err) {
      toast.error("Could not start the session", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent>
        <form onSubmit={(e) => void submit(e)} className="space-y-3">
          <DialogHeader>
            <DialogTitle>Start a session</DialogTitle>
            <DialogDescription>
              A new Claude Code session for {cook}, in a folder where it already has a station.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="start-pot" className="label-caps">Pot</Label>
            <Select value={activePot} onValueChange={setPot}>
              <SelectTrigger id="start-pot" size="sm" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {pots.map((x) => (
                  <SelectItem key={x} value={x}>
                    {x}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="start-dir" className="label-caps">Folder</Label>
            <Select value={activeDir} onValueChange={setDirectory}>
              <SelectTrigger id="start-dir" size="sm" className="w-full font-mono text-[12px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {dirs.map((d) => (
                  <SelectItem key={d} value={d} className="font-mono text-[12px]">
                    {d}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="start-task" className="label-caps">Task (optional)</Label>
            <Input id="start-task" value={task} onChange={(e) => setTask(e.target.value)} placeholder="PAY-12" className="font-mono" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="start-prompt" className="label-caps">Prompt (optional)</Label>
            <Textarea id="start-prompt" value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} />
          </div>
          <DialogFooter className="sm:items-center sm:justify-between">
            <div className="text-[12px] text-muted-foreground sm:max-w-[55%]">
              <a href={desktop} className="underline underline-offset-2 hover:text-foreground">
                Draft it in Claude Desktop instead
              </a>
              <p className="mt-1 text-[11.5px]">
                Desktop asks you to confirm the folder and to press Send. Run <code className="font-mono">corntchap-cli station scan</code> afterwards so corntchap can wake it.
              </p>
            </div>
            <Button type="submit" disabled={busy || !activePot || !activeDir}>
              {busy ? <Loader2 className="animate-spin" /> : <Play />} Start session
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** A cook's cloud runs, newest first: one station per Claude routine run, Devin or Coder session, webhook or GitHub-comment run. */
export function CloudRunList({ stations }: { stations: StationView[] }) {
  const now = useNow();
  const runs = [...stations].sort((a, b) => b.created_at.localeCompare(a.created_at));
  if (runs.length === 0) {
    return <p className="text-[12.5px] text-muted-foreground">No cloud runs yet. Each wake of a cloud target opens one, and what the run does is attributed to it.</p>;
  }
  return (
    <ul className="space-y-2">
      {runs.map((s) => {
        const live = !s.ended_at;
        return (
          <li key={s.id} className="rounded-xl border border-border bg-muted/40 px-3.5 py-3 text-[12.5px]">
            <div className="flex items-start gap-2">
              <p className="min-w-0 flex-1 truncate font-mono text-[12px] font-medium">
                <StationLink label={s.label ?? s.session} url={s.session_url} title={s.session_url ?? s.id} />
              </p>
              <span className={cn("label-caps shrink-0", live ? "text-primary" : "text-muted-foreground")}>{live ? "live" : "ended"}</span>
            </div>
            <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
              <span className="inline-flex items-center gap-1 text-muted-foreground">
                <SquareTerminal className="size-3.5" aria-hidden="true" /> {RUNTIME[s.target_type ?? s.runtime] ?? s.runtime}
              </span>
              <LabelChip className="font-mono">
                <Link to={`/pots/${s.project}/about`} className="hover:underline focus-visible:underline">{s.project}</Link>
              </LabelChip>
              <span className="font-mono text-[11px] text-muted-foreground" title={`Station ${s.id}`}>{s.id}</span>
            </div>
            {(s.holding.length > 0 || s.waiting.length > 0) && (
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                {s.holding.length > 0 && <span className="text-[11.5px] text-muted-foreground">holding</span>}
                {s.holding.map((t) => (
                  <TaskButton key={t} id={t} />
                ))}
                {s.waiting.map((t) => (
                  <TaskButton key={t} id={t}>
                    <Hourglass className="size-3 text-corn-text" aria-hidden="true" />
                  </TaskButton>
                ))}
              </div>
            )}
            <p className="mt-1.5 font-mono text-[10.5px] text-muted-foreground">
              started {fmtAgo(s.created_at, now)}
              {s.ended_at && ` · ended ${fmtAgo(s.ended_at, now)}`}
            </p>
          </li>
        );
      })}
    </ul>
  );
}
