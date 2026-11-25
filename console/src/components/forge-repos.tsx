import { useMemo, useState } from "react";
import { Link } from "react-router";
import { Archive, GitBranch, CloudDownload, Loader2, Lock, Search } from "lucide-react";
import { toast } from "sonner";
import type { ForgeRepoView } from "@corntchap/api";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorMessage } from "@/lib/api";
import { useProc, useProcCall } from "@/lib/data";
import { useDebounced } from "@/lib/use-debounced";
import { cn } from "@/lib/utils";

/**
 * "Add from GitHub": pick several of a pot's repositories from what the connected GitHub Apps can see.
 * The search text goes to the server (debounced) with a limit, so a big account does not ship all its repositories at once;
 * the list is also filtered here while the answer is on its way. The server marks those already in the pot.
 * A full-height sheet on a phone, a dialog beside it.
 */
/** Repositories asked for at a time. */
const SHOWN = 100;

export function AddFromGithubDialog({ project, open, onOpenChange }: { project: string; open: boolean; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  const [q, setQ] = useState("");
  const query = useDebounced(q.trim(), 300);
  const list = useProc("forgeRepos", { project, query: query || null, installation: null, limit: SHOWN }, { enabled: open, keepPrevious: true });
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const repos = useMemo(() => list.data?.repos ?? [], [list.data]);
  const needle = q.trim().toLowerCase();
  const visible = useMemo(() => repos.filter((r) => !needle || r.full_name.toLowerCase().includes(needle)), [repos, needle]);
  const groups = useMemo(() => {
    const m = new Map<string, ForgeRepoView[]>();
    for (const r of visible) m.set(r.account, [...(m.get(r.account) ?? []), r]);
    return [...m.entries()];
  }, [visible]);
  const selectable = visible.filter((r) => !r.in_pot).map((r) => r.full_name);
  const allOn = selectable.length > 0 && selectable.every((n) => picked.includes(n));
  const toggle = (n: string) => setPicked((p) => (p.includes(n) ? p.filter((x) => x !== n) : [...p, n]));
  const connected = list.data?.connected ?? true;

  function close(o: boolean) {
    if (!o) {
      setQ("");
      setPicked([]);
    }
    onOpenChange(o);
  }

  async function add() {
    setBusy(true);
    try {
      const r = await call("repoAddMany", { project, repos: picked.join("\n") });
      const skipped = r.skipped.length;
      toast.success(`${r.added.length} ${r.added.length === 1 ? "repository" : "repositories"} added to ${project}`, {
        description: skipped ? `${skipped} skipped: ${r.skipped.map((s) => s.input).join(", ")}` : undefined,
      });
      close(false);
    } catch (e) {
      toast.error("Could not add the repositories", { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent
        className="flex flex-col gap-3 overflow-hidden max-sm:h-dvh max-sm:max-h-dvh max-sm:rounded-none max-sm:pt-5 sm:h-[min(40rem,85dvh)] sm:max-w-2xl"
        aria-describedby="gh-pick-desc"
      >
        <DialogHeader className="text-left">
          <DialogTitle>Add from GitHub</DialogTitle>
          <DialogDescription id="gh-pick-desc">The repositories the connected GitHub Apps can see, most recently pushed first.</DialogDescription>
        </DialogHeader>

        {list.isLoading ? (
          <div className="grid flex-1 place-items-center text-muted-foreground" role="status" aria-label="Loading">
            <Loader2 className="animate-spin" />
          </div>
        ) : list.error ? (
          <p className="flex-1 text-[13px] text-destructive">{errorMessage(list.error)}</p>
        ) : !connected ? (
          <div className="flex-1 space-y-3 text-[13px] leading-snug">
            <p className="font-medium">No GitHub App is connected yet.</p>
            <p className="text-muted-foreground">An admin connects one on the Forges page and installs it on the accounts whose repositories should be offered here. You can still add a repository by its url.</p>
            <Button asChild variant="outline" size="sm">
              <Link to="/admin/forges" onClick={() => close(false)}>
                <CloudDownload /> Open the Forges page
              </Link>
            </Button>
          </div>
        ) : (
          <>
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
              <Input
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="Search repositories"
                aria-label="Search repositories"
                className="pl-9 text-[14px]"
                autoFocus
              />
            </div>
            {list.data?.errors.map((e) => (
              <p key={e} className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
                {e}
              </p>
            ))}
            <div className="flex items-center justify-between gap-3 text-[12.5px] text-muted-foreground">
              <span aria-live="polite">
                {visible.length} shown{list.data && list.data.total > repos.length ? ` of ${list.data.total} (type to narrow)` : ""}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={selectable.length === 0}
                onClick={() => setPicked((p) => (allOn ? p.filter((n) => !selectable.includes(n)) : [...new Set([...p, ...selectable])]))}
                className="h-8 pointer-coarse:h-11"
              >
                {allOn ? "Clear visible" : "Select all visible"}
              </Button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto rounded-[12px] border border-border" data-testid="gh-list">
              {groups.length === 0 && (
                <div className="p-4">
                  <p className="text-[13px] text-muted-foreground">{repos.length === 0 ? "The connected apps see no repositories. Install an app on an account from the Forges page." : "No repository matches."}</p>
                </div>
              )}
              {groups.map(([account, rs]) => (
                <section key={account} aria-label={account}>
                  <h3 className="label-caps sticky top-0 z-10 border-b border-border bg-muted px-3 py-1.5">
                    {account} <span className="font-normal normal-case">· {rs.length}</span>
                  </h3>
                  <ul className="divide-y divide-border">
                    {rs.map((r) => {
                      const on = r.in_pot || picked.includes(r.full_name);
                      return (
                        <li key={r.full_name}>
                          <label
                            className={cn(
                              "flex min-h-12 cursor-pointer items-start gap-3 px-3 py-2.5 hover:bg-muted/50 pointer-coarse:min-h-14",
                              r.in_pot && "cursor-default opacity-60 hover:bg-transparent",
                            )}
                          >
                            <Checkbox
                              checked={on}
                              disabled={r.in_pot}
                              onCheckedChange={() => toggle(r.full_name)}
                              aria-label={r.full_name}
                              className="mt-0.5 size-5 pointer-coarse:size-6"
                            />
                            <span className="min-w-0 flex-1">
                              <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                                <span className="font-mono text-[13px] font-semibold">{r.full_name}</span>
                                {r.private && (
                                  <span className="inline-flex items-center gap-1 rounded-full border border-border px-1.5 text-[11px] text-muted-foreground">
                                    <Lock className="size-3" aria-hidden="true" /> private
                                  </span>
                                )}
                                {r.archived && (
                                  <span className="inline-flex items-center gap-1 rounded-full border border-border px-1.5 text-[11px] text-muted-foreground">
                                    <Archive className="size-3" aria-hidden="true" /> archived
                                  </span>
                                )}
                                {r.default_branch && (
                                  <span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-muted-foreground">
                                    <GitBranch className="size-3" aria-hidden="true" /> {r.default_branch}
                                  </span>
                                )}
                              </span>
                              {r.in_pot ? (
                                <span className="mt-0.5 block text-[12px] text-muted-foreground">Already in this pot{r.pot_name ? ` as ${r.pot_name}` : ""}</span>
                              ) : (
                                r.description && <span className="mt-0.5 block truncate text-[12px] text-muted-foreground">{r.description}</span>
                              )}
                            </span>
                          </label>
                        </li>
                      );
                    })}
                  </ul>
                </section>
              ))}
            </div>
          </>
        )}

        <DialogFooter className="flex-row items-center justify-between gap-2 sm:justify-between">
          <span className="text-[13px] text-muted-foreground" aria-live="polite">
            {picked.length} selected
          </span>
          <span className="flex gap-2">
            <Button type="button" variant="ghost" onClick={() => close(false)}>
              Cancel
            </Button>
            <Button type="button" disabled={busy || picked.length === 0} onClick={() => void add()}>
              {busy && <Loader2 className="animate-spin" />} Add {picked.length || ""} {picked.length === 1 ? "repository" : "repositories"}
            </Button>
          </span>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
