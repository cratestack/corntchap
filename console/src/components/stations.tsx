import { useMemo, useState } from "react";
import { Link } from "react-router";
import type { ActivityView, StationView } from "@corntchap/api";
import { Activity, AppWindow, ChevronDown, Hourglass, Loader2, Play, SquareTerminal, Star, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useProcCall } from "@/lib/data";
import { staticSource, useStaticSource, type ListSource } from "@/lib/listbox";
import { useActivitySource, usePotsSource, useStationsSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { VirtualList } from "@/components/ui/virtual-list";
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

/** After the last row of a list that pages: loading, or the end of what the filters allow. */
function ListFooter({ source }: { source: ListSource<unknown> }) {
  if (source.isFetchingNextPage)
    return (
      <p role="status" className="flex items-center justify-center gap-2 py-3 text-[12px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
      </p>
    );
  if (source.capped) return <p className="py-3 text-center text-[12px] text-muted-foreground">That is as far as this goes.</p>;
  return null;
}

const NO_STATIONS: StationView[] = [];

/**
 * The stations to show: the paged `source` (stationPage), or, from a caller that still reads them whole, `stations`
 * (with `hasMore` / `onMore` for its own paging).
 */
function useStationSource(source: ListSource<StationView> | undefined, stations: StationView[] | undefined, more: { hasMore?: boolean | undefined; loadingMore?: boolean | undefined; onMore?: (() => void) | undefined }) {
  const fixed = useStaticSource(stations ?? NO_STATIONS);
  const { hasMore = false, loadingMore = false, onMore } = more;
  return useMemo<ListSource<StationView>>(
    () => source ?? (hasMore ? { ...staticSource(stations ?? NO_STATIONS), hasNextPage: true, isFetchingNextPage: loadingMore, fetchNextPage: () => onMore?.() } : fixed),
    [source, stations, fixed, hasMore, loadingMore, onMore],
  );
}

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

/**
 * What the station's agent last did (from its hooks): a one-line summary, and the rest of its recent calls with
 * checkpoints in a dialog (paged, virtualized).
 */
function StationActivity({ station, now }: { station: StationView; now: number }) {
  const [open, setOpen] = useState(false);
  const activity = useActivitySource(station.project, { station: station.id });
  const last = activity.items[0];
  if (!last) return null;
  return (
    <div className="mt-1.5 text-[11.5px]">
      <button
        type="button"
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
        className="flex w-full items-center gap-1.5 rounded-md text-left text-muted-foreground hover:text-foreground focus-visible:text-foreground"
      >
        <Activity className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono" title={last.summary ?? activityLine(last)}>
          {activityLine(last)}
        </span>
        <span className="shrink-0 font-mono text-[10.5px]">{fmtAgo(last.at, now)}</span>
        <ChevronDown className="size-3.5 shrink-0 -rotate-90" aria-hidden="true" />
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="flex max-h-[80dvh] flex-col sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>Recent activity</DialogTitle>
            <DialogDescription className="font-mono text-[11.5px]">{station.label ?? station.directory ?? station.session}</DialogDescription>
          </DialogHeader>
          <div className="h-[60dvh] min-h-0">
            <VirtualList
              items={activity.items}
              itemKey={(a) => a.seq}
              scroll="own"
              onEndReached={() => {
                if (activity.hasNextPage && !activity.isFetchingNextPage) activity.fetchNextPage();
              }}
              footer={<ListFooter source={activity} />}
              itemClassName="border-b border-border/60 py-1.5 text-[12px] last:border-b-0"
              aria-label="Recent activity"
              renderItem={(a) => (
                <div className="flex items-baseline gap-2">
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
                </div>
              )}
            />
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** A cook's registered agent sessions. Registration happens in the CLI; the console shows and removes them. */
export function StationList({
  source,
  stations,
  hasMore,
  loadingMore,
  onMore,
  cook,
  kind,
}: {
  /** The cook's stations as a paged read (`useStationsSource`). */
  source?: ListSource<StationView>;
  /** Or already read whole, with `hasMore` / `loadingMore` / `onMore` when the caller pages them itself. */
  stations?: StationView[];
  hasMore?: boolean;
  loadingMore?: boolean;
  onMore?: () => void;
  cook?: string;
  kind?: string | null;
}) {
  const list = useStationSource(source, stations, { hasMore, loadingMore, onMore });
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
  // a session can start where one of the stations loaded has a folder (more may load as the list is read)
  const startable = list.items.some((s) => !!s.directory) || list.hasNextPage;

  return (
    <div className="space-y-3">
      {canStart && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" className="max-sm:w-full" disabled={!startable} onClick={() => setStarting(true)}>
            <Play /> Start a session
          </Button>
          {!startable && !list.isLoading && (
            <span className="text-[12px] text-muted-foreground">
              Register a session first: run <code className="font-mono text-[11.5px]">corntchap-cli station start</code> or <code className="font-mono text-[11.5px]">station scan</code> in the repo.
            </span>
          )}
        </div>
      )}
      {canStart && <StartSessionDialog open={starting} onOpenChange={setStarting} cook={cook} />}
      {list.isLoading && list.items.length === 0 ? (
        <p role="status" className="text-[12.5px] text-muted-foreground">
          Loading stations…
        </p>
      ) : list.items.length === 0 ? (
        <p className="text-[12.5px] text-muted-foreground">
          No stations yet. Run <code className="font-mono text-[11.5px]">corntchap-cli station scan</code> in a repo to register this cook’s sessions.
        </p>
      ) : (
        <>
          <p className="text-[12px] text-muted-foreground">
            The fire wakes each station’s session directly, so work resumes where it left off. The main station <Star className="inline size-3 align-[-1px]" aria-hidden="true" /> answers messages and mentions in its pot.
          </p>
          <VirtualList<StationView>
            items={list.items}
            itemKey={(s) => s.id}
            scroll="window"
            onEndReached={() => {
              if (list.hasNextPage && !list.isFetchingNextPage) list.fetchNextPage();
            }}
            footer={<ListFooter source={list} />}
            itemClassName="pb-2"
            defaultItemHeight={120}
            aria-label="Stations"
            renderItem={(s) => {
              const where = s.label ?? s.directory ?? s.session;
              return (
                <div className={cn("rounded-xl border bg-muted/40 px-3.5 py-3 text-[12.5px]", s.main ? "border-primary/50" : "border-border")}>
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
                </div>
              );
            }}
          />
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
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  cook: string;
}) {
  const call = useProcCall();
  const [pot, setPot] = useState("");
  const [directory, setDirectory] = useState("");
  const [task, setTask] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const potQuery = useStoreValue("");
  const dirQuery = useStoreValue("");

  // the pot: the one picked, else where the cook's newest station with a folder is
  const newest = useStationsSource({ cook, kind: "session", enabled: open });
  const firstPot = newest.items.find((s) => !!s.directory)?.project ?? "";
  const activePot = pot || firstPot;
  const pots = usePotsSource({ q: potQuery.value, enabled: open });
  // the folders: where the cook has a station in that pot, the main station's first (as far as they are loaded)
  const inPot = useStationsSource({ project: activePot, cook, kind: "session", q: dirQuery.value, enabled: open && !!activePot });
  const dirs = useMemo<ListSource<StationView>>(() => {
    const seen = new Set<string>();
    const withDir = inPot.items.filter((s) => !!s.directory);
    const items = [...withDir.filter((s) => s.main), ...withDir].filter((s) => (seen.has(s.directory!) ? false : !!seen.add(s.directory!)));
    return { ...inPot, items };
  }, [inPot]);
  const activeDir = directory || (dirQuery.value ? "" : (dirs.items[0]?.directory ?? ""));

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
            <VirtualCombobox<(typeof pots.items)[number]>
              id="start-pot"
              label="Pot"
              source={pots}
              onQuery={potQuery.set}
              itemKey={(x) => x.key}
              itemText={(x) => x.key}
              value={activePot || null}
              onChange={(v) => {
                setPot(v ?? "");
                setDirectory("");
              }}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="start-dir" className="label-caps">Folder</Label>
            <VirtualCombobox<StationView>
              id="start-dir"
              label="Folder"
              placeholder={dirs.isLoading ? "Loading…" : "No station with a folder in this pot"}
              source={dirs}
              onQuery={dirQuery.set}
              itemKey={(x) => x.directory!}
              itemText={(x) => x.directory!}
              renderItem={(x) => <span className="block truncate font-mono text-[12px]">{x.directory}</span>}
              value={activeDir || null}
              onChange={(v) => setDirectory(v ?? "")}
              className="font-mono text-[12px]"
            />
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
export function CloudRunList({
  source,
  stations,
  hasMore = false,
  loadingMore = false,
  onMore,
}: {
  /** The cook's cloud runs as a paged read (`useStationsSource` with `kind: "cloud-run"`). */
  source?: ListSource<StationView>;
  /** Or the runs already read, with `hasMore` / `loadingMore` / `onMore` when the caller pages them itself. */
  stations?: StationView[];
  hasMore?: boolean;
  loadingMore?: boolean;
  onMore?: () => void;
}) {
  const now = useNow();
  const list = useStationSource(source, stations, { hasMore, loadingMore, onMore });
  if (list.items.length === 0) {
    return <p className="text-[12.5px] text-muted-foreground">No cloud runs yet. Each wake of a cloud target opens one, and what the run does is attributed to it.</p>;
  }
  return (
    <VirtualList<StationView>
      items={list.items}
      itemKey={(s) => s.id}
      scroll="window"
      onEndReached={() => {
        if (list.hasNextPage && !list.isFetchingNextPage) list.fetchNextPage();
      }}
      footer={<ListFooter source={list} />}
      itemClassName="pb-2"
      defaultItemHeight={110}
      aria-label="Cloud runs"
      renderItem={(s) => {
        const live = !s.ended_at;
        return (
          <div className="rounded-xl border border-border bg-muted/40 px-3.5 py-3 text-[12.5px]">
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
          </div>
        );
      }}
    />
  );
}
