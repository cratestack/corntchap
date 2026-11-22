import { useState } from "react";
import type { StationView } from "@corntchap/api";
import { Hourglass, Loader2, SquareTerminal, Star, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { Button } from "@/components/ui/button";
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
import { IdChip, LabelChip } from "./bits";

const RUNTIME: Record<string, string> = {
  opencode: "OpenCode",
  "claude-code": "Claude Code",
  codex: "Codex",
  other: "Other",
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

/** A cook's registered agent sessions. Registration happens in the CLI; the console shows and removes them. */
export function StationList({ stations }: { stations: StationView[] }) {
  const call = useProcCall();
  const now = useNow();
  const [removing, setRemoving] = useState<StationView | null>(null);
  const [busy, setBusy] = useState(false);

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

  return (
    <div className="mt-2 space-y-2">
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
                <li key={s.id} className={cn("rounded-lg border bg-muted/40 px-3 py-2 text-[12.5px]", s.main ? "border-primary/50" : "border-border")}>
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
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      aria-pressed={s.main}
                      aria-label={s.main ? `${where} is the main station of ${s.project}: unset it` : `Make ${where} the main station of ${s.project}`}
                      title={s.main ? "Main station: answers messages and mentions (click to unset)" : "Make it the main station: it answers messages and mentions"}
                      onClick={() => void setMain(s, !s.main)}
                    >
                      <Star className={cn(s.main && "fill-primary text-primary")} />
                    </Button>
                    <Button size="icon-xs" variant="ghost" aria-label={`Remove station ${where}`} onClick={() => setRemoving(s)}>
                      <Trash2 />
                    </Button>
                  </div>
                  <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
                    <span className="inline-flex items-center gap-1 text-muted-foreground">
                      <SquareTerminal className="size-3.5" aria-hidden="true" /> {RUNTIME[s.runtime] ?? s.runtime}
                    </span>
                    <LabelChip className="font-mono">{s.project}</LabelChip>
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
