import { useMemo, useState, type FormEvent } from "react";
import { GitBranch, CloudDownload, Link2, Loader2, Plus, Trash2, Unlink } from "lucide-react";
import { toast } from "sonner";
import type { ProjectSummary, RepoView } from "@corntchap/api";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { errorMessage } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useProcCall } from "@/lib/data";
import { useReposSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { Group, GroupRow, RepoChip } from "./bits";
import { AddFromGithubDialog } from "./forge-repos";
import { FilterBar } from "./filter-bar";
import { listFooter, loadMore } from "./list-footer";
import { VirtualCombobox } from "./ui/virtual-combobox";
import { VirtualList } from "./ui/virtual-list";

const repoKey = (r: RepoView) => r.name;
const repoText = (r: RepoView) => r.name;
const repoRow = (r: RepoView) => (
  <span className="flex min-w-0 items-center gap-2" title={r.url}>
    <GitBranch className="size-3.5 shrink-0 text-primary" aria-hidden="true" />
    <span className="truncate font-mono text-[12.5px]">{r.name}</span>
  </span>
);

/**
 * A pot's repositories as a searchable, paged picker: one repository (`value` a name or null) or several. For a task's
 * repositories, a cook's scope, or the repository a dependency points at. Pass `skip` to leave one out.
 */
export function RepoPicker(
  props: { pot: string; label: string; id?: string | undefined; skip?: string | undefined; placeholder?: string | undefined; disabled?: boolean | undefined } & (
    | { multiple: true; value: string[]; onChange: (v: string[]) => void }
    | { multiple?: false; value: string | null; onChange: (v: string | null) => void }
  ),
) {
  const { pot, label, id, skip, placeholder, disabled } = props;
  const q = useStoreValue("");
  const source = useReposSource(pot, q.value);
  const rows = useMemo(() => (skip ? { ...source, items: source.items.filter((r) => r.name !== skip) } : source), [source, skip]);
  const common = { source: rows, onQuery: q.set, itemKey: repoKey, itemText: repoText, renderItem: repoRow, label, id, placeholder, disabled };
  return props.multiple ? (
    <VirtualCombobox<RepoView> {...common} multiple value={props.value} onChange={props.onChange} />
  ) : (
    <VirtualCombobox<RepoView> {...common} value={props.value} onChange={props.onChange} />
  );
}

/** One repository of the list: its url and branch, what it depends on, and (for managers) the link and remove buttons. */
function RepoRow({
  repo: r,
  canManage,
  hasOthers,
  onLink,
  onRemove,
  onUnlink,
}: {
  repo: RepoView;
  canManage: boolean;
  hasOthers: boolean;
  onLink: () => void;
  onRemove: () => void;
  onUnlink: (on: string) => void;
}) {
  return (
    <div className="px-4 py-3">
      <div className="flex min-w-0 items-start gap-2">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px] font-medium leading-tight">
            <span className="font-mono font-bold">{r.name}</span>
            {r.default_branch && (
              <span className="inline-flex items-center gap-1 font-mono text-[11.5px] font-normal text-muted-foreground">
                <GitBranch className="size-3" aria-hidden="true" /> {r.default_branch}
              </span>
            )}
          </p>
          <p className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground" title={r.url}>
            {r.url.replace(/^https?:\/\//, "")}
          </p>
          {r.depends_on.length > 0 && (
            <p className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[12px] text-muted-foreground">
              depends on
              {r.depends_on.map((d) => (
                <span key={d.name} className="inline-flex items-center gap-1">
                  <RepoChip>
                    {d.name}
                    <span className="font-sans font-normal text-muted-foreground"> · {d.mode === "task" ? "follow-up task" : "tell cooks"}</span>
                  </RepoChip>
                  {canManage && (
                    <button
                      type="button"
                      aria-label={`${r.name} no longer depends on ${d.name}`}
                      onClick={() => onUnlink(d.name)}
                      className="grid size-6 place-items-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground pointer-coarse:size-9"
                    >
                      <Unlink className="size-3.5" />
                    </button>
                  )}
                </span>
              ))}
            </p>
          )}
        </div>
        {canManage && (
          <div className="flex shrink-0 items-center">
            {hasOthers && (
              <Button size="icon-sm" variant="ghost" aria-label={`${r.name} depends on…`} title="Depends on…" onClick={onLink}>
                <Link2 />
              </Button>
            )}
            <Button size="icon-sm" variant="ghost" aria-label={`Remove ${r.name}`} onClick={onRemove}>
              <Trash2 />
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

/** The repositories of a pot, with what each depends on: searchable and paged. Owners and admins add, remove and link. */
export function PotRepos({ project, canManage }: { project: ProjectSummary; canManage: boolean }) {
  const call = useProcCall();
  const [dlg, setDlg] = useState<{ kind: "add" } | { kind: "github" } | { kind: "link"; repo: RepoView } | { kind: "remove"; repo: RepoView } | null>(null);
  const q = useStoreValue("");
  const source = useReposSource(project.key, q.value);
  const repos = source.items;
  const none = repos.length === 0 && !source.isLoading && !source.pending && !q.value.trim();

  async function unlink(repo: string, on: string) {
    try {
      await call("repoUnlink", { project: project.key, repo, on });
      toast.success(`${repo} no longer depends on ${on}`);
    } catch (e) {
      toast.error("Could not remove the link", { description: errorMessage(e) });
    }
  }

  return (
    <>
      <Group
        title="Repositories"
        hint={
          none
            ? "None yet: the pot works without. Add them so tasks can say which repository they are about, and cooks can handle some of them."
            : "Tasks name them in repos; a cook can handle some of them. A repository can depend on another: serving a task in the upstream tells the dependent's cooks."
        }
        aside={
          canManage ? (
            <span className="flex flex-wrap gap-1.5">
              <Button size="sm" variant="outline" onClick={() => setDlg({ kind: "github" })} className="h-8 pointer-coarse:h-11">
                <CloudDownload /> Add from GitHub
              </Button>
              <Button size="sm" variant="outline" onClick={() => setDlg({ kind: "add" })} className="h-8 pointer-coarse:h-11">
                <Plus /> Add by URL
              </Button>
            </span>
          ) : undefined
        }
      >
        {(!none || project.repos.length > 0) && (
          <div className="p-3">
            <FilterBar q={q.value} onQ={q.set} facets={[]} values={{}} onChange={() => {}} onClear={() => q.set("")} searchPlaceholder="Search repositories" />
          </div>
        )}
        {source.isLoading && repos.length === 0 && <GroupRow label="Loading repositories" />}
        {!!source.error && repos.length === 0 && <GroupRow label="The repositories would not load" hint={errorMessage(source.error)} />}
        {none && !source.error && <GroupRow label="No repository" hint={canManage ? "Add the first one with the button above." : "A team owner can add them."} />}
        {!none && !source.isLoading && !source.pending && repos.length === 0 && !source.error && <GroupRow label="No repository matches that" />}
        {repos.length > 0 && (
          <VirtualList<RepoView>
            items={repos}
            itemKey={repoKey}
            scroll="window"
            aria-label="Repositories"
            onEndReached={loadMore(source)}
            footer={listFooter(source)}
            itemClassName="border-b border-border last:border-b-0"
            defaultItemHeight={72}
            renderItem={(r) => (
              <RepoRow
                repo={r}
                canManage={canManage}
                hasOthers={project.repos.length > 1}
                onLink={() => setDlg({ kind: "link", repo: r })}
                onRemove={() => setDlg({ kind: "remove", repo: r })}
                onUnlink={(on) => void unlink(r.name, on)}
              />
            )}
          />
        )}
      </Group>
      <AddFromGithubDialog project={project.key} open={dlg?.kind === "github"} onOpenChange={(o) => !o && setDlg(null)} />
      <AddRepoDialog project={project.key} open={dlg?.kind === "add"} onOpenChange={(o) => !o && setDlg(null)} />
      {dlg?.kind === "link" && <LinkDialog project={project} repo={dlg.repo} onOpenChange={(o) => !o && setDlg(null)} />}
      {dlg?.kind === "remove" && <RemoveDialog project={project.key} repo={dlg.repo} onOpenChange={(o) => !o && setDlg(null)} />}
    </>
  );
}

function AddRepoDialog({ project, open, onOpenChange }: { project: string; open: boolean; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await call("repoAdd", { project, url: url.trim(), name: name.trim() || null, defaultBranch: branch.trim() || null });
      toast.success(`${r.added.name} added to ${project}`);
      setUrl("");
      setName("");
      setBranch("");
      onOpenChange(false);
    } catch (err) {
      toast.error("Could not add the repository", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader className="text-left">
          <DialogTitle>Add a repository</DialogTitle>
          <DialogDescription>Stations whose checkout has this origin are matched to it.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="repo-url">Git url</Label>
            <Input id="repo-url" required autoFocus value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://github.com/acme/backend.git" className="font-mono text-[13px]" />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="repo-name">Name</Label>
              <Input id="repo-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="backend" className="font-mono text-[13px]" />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="repo-branch">Default branch</Label>
              <Input id="repo-branch" value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="main" className="font-mono text-[13px]" />
            </div>
          </div>
          <p className="-mt-2 text-[12px] text-muted-foreground">The name is the short alias tasks and cooks use; left empty it is the last part of the url.</p>
          <DialogFooter className="flex-row justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} className="max-sm:flex-1">
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !url.trim()} className="max-sm:flex-[2]">
              {busy && <Loader2 className="animate-spin" />} Add
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function LinkDialog({ project, repo, onOpenChange }: { project: ProjectSummary; repo: RepoView; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  const [on, setOn] = useState<string | null>(null);
  const [mode, setMode] = useState("notify");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!on) return;
    setBusy(true);
    try {
      await call("repoLink", { project: project.key, repo: repo.name, on, mode });
      toast.success(`${repo.name} now depends on ${on}`);
      onOpenChange(false);
    } catch (err) {
      toast.error("Could not link", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader className="text-left">
          <DialogTitle>{repo.name} depends on…</DialogTitle>
          <DialogDescription>When a task in that repository is served, this one hears about it. One hop: it does not chain.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="link-on">Repository</Label>
            <RepoPicker id="link-on" pot={project.key} skip={repo.name} label="Repository" placeholder="Choose a repository" value={on} onChange={setOn} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="link-mode">Then</Label>
            <Select value={mode} onValueChange={setMode}>
              <SelectTrigger id="link-mode" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="notify">Tell the cooks that handle {repo.name}</SelectItem>
                <SelectItem value="task">Open a follow-up task for {repo.name}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <DialogFooter className="flex-row justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} className="max-sm:flex-1">
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !on} className="max-sm:flex-[2]">
              {busy && <Loader2 className="animate-spin" />} Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function RemoveDialog({ project, repo, onOpenChange }: { project: string; repo: RepoView; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  const [busy, setBusy] = useState(false);
  async function remove() {
    setBusy(true);
    try {
      await call("repoRemove", { project, name: repo.name });
      toast.success(`${repo.name} removed from ${project}`);
      onOpenChange(false);
    } catch (err) {
      toast.error("Could not remove it", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader className="text-left">
          <DialogTitle>Remove {repo.name}?</DialogTitle>
          <DialogDescription>
            Open tasks and cook scopes stop naming it. A task left with no repository is for any cook. Served tasks keep their history.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className="flex-row justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)} className="max-sm:flex-1">
            Cancel
          </Button>
          <Button variant="destructive" disabled={busy} onClick={() => void remove()} className="max-sm:flex-[2]">
            {busy && <Loader2 className="animate-spin" />} Remove
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Per pot with repositories: the ones a cook handles (all, or a pick). */
export function CookRepoScopes({
  cook,
  pots,
  scopes,
}: {
  cook: string;
  pots: ProjectSummary[];
  scopes: { project: string; repos: string[] }[];
}) {
  const withRepos = pots.filter((p) => p.repos.length > 0);
  const [editing, setEditing] = useState<ProjectSummary | null>(null);
  if (withRepos.length === 0) return null;
  return (
    <>
      <Group
        title="Repositories"
        hint={`Which repositories of a pot ${cook} handles. claim_next gives it only tasks about those, or about none; the fire wakes it only for those.`}
      >
        {withRepos.map((p) => {
          const mine = scopes.find((s) => s.project === p.key)?.repos ?? [];
          return (
            <GroupRow
              key={p.key}
              label={<span className="font-mono font-bold">{p.key}</span>}
              hint={
                mine.length === 0 ? (
                  "All repositories"
                ) : (
                  <span className="mt-1 flex flex-wrap gap-1">
                    {mine.map((m) => (
                      <RepoChip key={m}>{m}</RepoChip>
                    ))}
                  </span>
                )
              }
              onClick={() => setEditing(p)}
            />
          );
        })}
      </Group>
      {editing && (
        <ScopeDialog
          cook={cook}
          pot={editing}
          initial={scopes.find((s) => s.project === editing.key)?.repos ?? []}
          onOpenChange={(o) => !o && setEditing(null)}
        />
      )}
    </>
  );
}

function ScopeDialog({ cook, pot, initial, onOpenChange }: { cook: string; pot: ProjectSummary; initial: string[]; onOpenChange: (o: boolean) => void }) {
  const call = useProcCall();
  const [all, setAll] = useState(initial.length === 0);
  const [pick, setPick] = useState<string[]>(initial);
  const [busy, setBusy] = useState(false);
  async function save() {
    setBusy(true);
    try {
      await call("cookSetRepos", { project: pot.key, cook, repos: all ? "" : pick.join(",") });
      toast.success(all || pick.length === 0 ? `${cook} handles every repository of ${pot.key}` : `${cook} handles ${pick.join(", ")} in ${pot.key}`);
      onOpenChange(false);
    } catch (err) {
      toast.error("Could not save", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader className="text-left">
          <DialogTitle>
            {cook} in {pot.key}
          </DialogTitle>
          <DialogDescription>Tasks about none of the chosen repositories are not given to it by claim_next. Tasks about no repository always are.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div role="radiogroup" aria-label="Scope" className="grid grid-cols-2 gap-2">
            {[
              { v: true, label: "All repositories" },
              { v: false, label: "Only some" },
            ].map((o) => (
              <button
                key={String(o.v)}
                type="button"
                role="radio"
                aria-checked={all === o.v}
                onClick={() => setAll(o.v)}
                className={cn(
                  "min-h-11 rounded-lg border px-3 text-[13px] font-medium",
                  all === o.v ? "border-primary bg-primary/10" : "border-border bg-card text-muted-foreground hover:bg-muted/60",
                )}
              >
                {o.label}
              </button>
            ))}
          </div>
          {!all && <RepoPicker multiple pot={pot.key} value={pick} onChange={setPick} label={`Repositories ${cook} handles`} />}
        </div>
        <DialogFooter className="flex-row justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)} className="max-sm:flex-1">
            Cancel
          </Button>
          <Button disabled={busy || (!all && pick.length === 0)} onClick={() => void save()} className="max-sm:flex-[2]">
            {busy && <Loader2 className="animate-spin" />} Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
