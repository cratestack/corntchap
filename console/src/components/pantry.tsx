import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useLocation, useSearchParams } from "react-router";
import type { MemoryRememberArgs, MemoryView, TagCount } from "@corntchap/api";
import { ChevronRight, Link2, Loader2, Pencil, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall } from "@/lib/data";
import { QUERY_DEBOUNCE_MS } from "@/lib/list-query";
import { useMappedSource, type ListSource } from "@/lib/listbox";
import { staticFacet } from "@/lib/people";
import { useMemoriesSource, useTagsSource } from "@/lib/sources";
import { useUrlFilters, type UrlFilterSchema } from "@/lib/url-filters";
import { useDebounced } from "@/lib/use-debounced";
import { useStoreValue } from "@/lib/use-store-value";
import { useTagOptions } from "@/lib/pot-pickers";
import { errorMessage, errorStatus } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
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
import { Avatar, LabelChip, SectionLabel } from "./bits";
import { RefreshButton } from "./refresh-button";
import { Markdown } from "./markdown";
import { TagInput } from "./tag-input";
import { FilterBar, type FacetDef, type FacetOption } from "./filter-bar";
import { ListPage } from "./list-page";

// ---------------------------------------------------------------- kinds

export const KINDS = [
  { id: "note", label: "Note", dot: "bg-muted-foreground/60", hint: "anything worth keeping" },
  { id: "convention", label: "Convention", dot: "bg-primary", hint: "how we do it here" },
  { id: "gotcha", label: "Gotcha", dot: "bg-corn", hint: "a trap, and the way around it" },
  { id: "decision", label: "Decision", dot: "bg-[#3d4590] dark:bg-[#8e97ec]", hint: "what was chosen, and why" },
  { id: "howto", label: "How-to", dot: "bg-success", hint: "build, test, deploy, run" },
  { id: "fact", label: "Fact", dot: "bg-[#8a3b66] dark:bg-[#d98bb4]", hint: "something true about the system" },
] as const;

const kindOf = (id: string) => KINDS.find((k) => k.id === id) ?? KINDS[0];

function KindDot({ kind, className }: { kind: string; className?: string }) {
  return <span aria-hidden="true" className={cn("inline-block size-2.5 shrink-0 rounded-full", kindOf(kind).dot, className)} />;
}

/** The first couple of lines of a markdown body as plain text, for the row excerpt. */
function plainExcerpt(md: string): string {
  return md
    .replace(/```[a-z]*\n?/gi, "")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+\.)\s+/gm, "")
    .replace(/[*`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

// ---------------------------------------------------------------- the view

/** The most memories a search answers (it is ranked by relevance, not paged). */
const LIMIT = 100;

/** The pantry's filters in the URL: the search text (`q`), a kind and the tags a memory must all carry. */
const PANTRY_FILTERS = { kind: { type: "string" }, tag: { type: "list" } } as const satisfies UrlFilterSchema;
const KIND_FACET = staticFacet("kind", "Kind", KINDS.map((k) => ({ value: k.id, label: k.label, hint: k.hint })));
const memoryKey = (m: MemoryView) => m.key;

export function PantryView({ potKey, onNew }: { potKey: string; onNew: () => void }) {
  const [params, setParams] = useSearchParams();
  const loc = useLocation();
  const f = useUrlFilters(PANTRY_FILTERS);
  const tags = f.values.tag;
  const kind = f.values.kind;
  const selected = params.get("memory");
  const searching = !!f.q.trim();
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if ((loc.state as { focus?: boolean } | null)?.focus) searchRef.current?.focus();
  }, [loc.state]);

  // Without a search the pantry is read a page at a time, newest change first (kind and tags narrow it on the server);
  // a search is ranked by relevance and answers its best matches.
  const paged = useMemoriesSource(potKey, { kind: kind || null, tags: tags.length ? tags.join(",") : null, enabled: !searching });
  const query = useDebounced(f.q, QUERY_DEBOUNCE_MS).trim();
  const found = useProc(
    "memoryRecall",
    { project: potKey, query: query || null, kind: kind || null, tags: tags.length ? tags.join(",") : null, limit: LIMIT },
    { enabled: searching && !!query },
  );
  const recalled = useMemo<ListSource<MemoryView>>(() => {
    const items = found.data ?? [];
    return { items, hasNextPage: false, fetchNextPage: () => {}, isLoading: found.isLoading, isFetchingNextPage: false, error: found.error, capped: items.length >= LIMIT, pending: f.q.trim() !== query };
  }, [found.data, found.isLoading, found.error, f.q, query]);
  const source = searching ? recalled : paged;

  const setMemory = (key: string | null, push: boolean) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (key) n.set("memory", key);
        else n.delete("memory");
        return n;
      },
      { replace: !push },
    );

  const tagFacet = useTagFacet(potKey);
  const filtered = f.active > 0 || searching;

  return (
    <>
      <ListPage<MemoryView>
        embedded
        title="Pantry"
        className="max-w-5xl max-md:pb-16"
        filters={
          <FilterBar
            q={f.q}
            onQ={f.setQ}
            searchRef={searchRef}
            facets={[KIND_FACET, tagFacet]}
            values={{ kind: f.values.kind, tag: f.values.tag }}
            onChange={(p) => f.set(p as never)}
            onClear={f.clear}
            searchPlaceholder="Search keys, titles and bodies…"
          />
        }
        source={source}
        itemKey={memoryKey}
        renderItem={(m) => <MemoryRow m={m} selected={selected === m.key} onOpen={(key) => setMemory(key, false)} />}
        empty={
          filtered ? (
            <>
              Nothing on the shelf matches that. Try fewer words, or loosen a kind or tag. Search looks at keys, titles and bodies.{" "}
              <Button variant="link" className="h-auto p-0" onClick={f.clear}>
                Clear filters
              </Button>
            </>
          ) : (
            <>
              Nothing in the pantry yet. Agents stock it as they learn the codebase: conventions, gotchas, how to run things. The next cook reads it before starting, so nobody works out the same thing twice.{" "}
              <Button variant="link" className="h-auto p-0" onClick={onNew}>
                Stock the pantry
              </Button>
            </>
          )
        }
      />

      <Button
        type="button"
        onClick={onNew}
        aria-label="Stock the pantry"
        className="bottom-tabbar fixed right-4 z-30 mb-4 h-12 rounded-full px-5 shadow-lg md:hidden"
      >
        <Plus /> Stock
      </Button>

      <MemorySheet potKey={potKey} memoryKey={selected} onClose={() => setMemory(null, true)} />
    </>
  );
}

/** The pantry's tags (with how many memories carry each), searched and paged by the server; a memory must have all that are chosen. */
function useTagFacet(pot: string): FacetDef {
  const q = useStoreValue("");
  const source = useMappedSource(useTagsSource(pot, { q: q.value }), tagOption);
  return { id: "tag", label: "Tag", source, onQuery: q.set, multiple: true };
}
const tagOption = (t: TagCount): FacetOption => ({ value: t.tag, label: t.tag, hint: String(t.count) });

function MemoryRow({ m, selected, onOpen }: { m: MemoryView; selected: boolean; onOpen: (key: string) => void }) {
  const now = useNow();
  const who = m.updated_by ?? m.created_by;
  const excerpt = plainExcerpt(m.body);
  return (
    <button
      type="button"
      onClick={() => onOpen(m.key)}
      aria-label={`${m.title} (${kindOf(m.kind).label}, ${m.key})`}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "group block w-full min-w-0 px-3.5 py-3 text-left transition-colors hover:bg-muted/50 focus-visible:bg-muted/60 active:bg-muted/70 md:px-4",
        selected && "bg-primary/5",
      )}
    >
      <div className="flex items-center gap-2">
        <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-muted/60 px-2 py-0.5 text-[11.5px] font-medium">
          <KindDot kind={m.kind} className="size-2" /> {kindOf(m.kind).label}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">{m.key}</span>
        <span className="font-mono text-[10.5px] text-muted-foreground" title="Revision">
          r{m.revision}
        </span>
        <ChevronRight className="hidden size-4 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 sm:block" aria-hidden="true" />
      </div>
      <h3 className="mt-2 font-display text-[16px] leading-snug">{m.title}</h3>
      {excerpt && <p className="mt-1 line-clamp-3 text-[13.5px] leading-snug text-muted-foreground">{excerpt}</p>}
      <div className="mt-2.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
        {who && <Avatar name={who} className="size-5 text-[10px]" />}
        <span className="text-[12px] text-muted-foreground">
          {who ? `${who} · ` : ""}
          {fmtAgo(m.updated_at, now)}
        </span>
        {m.tags.map((t) => (
          <LabelChip key={t}>{t}</LabelChip>
        ))}
      </div>
    </button>
  );
}

// ---------------------------------------------------------------- the sheet

export function MemorySheet({ potKey, memoryKey, onClose }: { potKey: string; memoryKey: string | null; onClose: () => void }) {
  return (
    <Sheet open={!!memoryKey} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right" className="w-full gap-0 overflow-y-auto p-0 sm:max-w-[34rem]" aria-describedby="memory-sheet-desc">
        <SheetTitle className="sr-only">Memory {memoryKey}</SheetTitle>
        <SheetDescription id="memory-sheet-desc" className="sr-only">
          The memory in full, with its details, and actions to edit or forget it
        </SheetDescription>
        {memoryKey && <MemoryBody key={memoryKey} potKey={potKey} memoryKey={memoryKey} onClose={onClose} />}
      </SheetContent>
    </Sheet>
  );
}

function MemoryBody({ potKey, memoryKey, onClose }: { potKey: string; memoryKey: string; onClose: () => void }) {
  const q = useProc("memoryGet", { project: potKey, key: memoryKey });
  const call = useProcCall();
  const [, setParams] = useSearchParams();
  const now = useNow();
  // the copy the form started from: its revision is what a save is checked against, even as live data moves on
  const [base, setBase] = useState<MemoryView | null>(null);
  const editing = base !== null;
  const [confirm, setConfirm] = useState(false);
  const [notifyForget, setNotifyForget] = useState(false);
  const [busy, setBusy] = useState(false);
  const m = q.data;

  if (q.error && !m) {
    return (
      <div className="p-8">
        <h2 className="text-2xl">That memory is not on the shelf</h2>
        <p className="mt-2 text-muted-foreground">{errorMessage(q.error)}</p>
        <p className="mt-1 text-[13px] text-muted-foreground">It may have been forgotten since you last looked.</p>
      </div>
    );
  }
  if (!m) {
    return (
      <div className="space-y-4 p-6" aria-busy="true" aria-label="Loading memory">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="h-8 w-3/4" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  const kind = kindOf(m.kind);
  const openTask = (id: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.delete("memory");
        n.set("task", id);
        return n;
      },
      { replace: false },
    );

  async function forget() {
    setBusy(true);
    try {
      await call("memoryForget", { project: potKey, key: memoryKey, notify: notifyForget || null, station: null });
      toast.success("Forgotten", { description: m!.title });
      setConfirm(false);
      onClose();
    } catch (e) {
      toast.error("Could not forget it", { description: errorMessage(e) });
      setConfirm(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <header className="relative border-b border-border px-6 pt-6 pb-4">
        <RefreshButton size="icon-sm" className="absolute top-3 right-11 size-7" />
        <div className="flex flex-wrap items-center gap-2.5 pr-16">
          <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-muted/60 px-2 py-0.5 text-[11.5px] font-medium">
            <KindDot kind={m.kind} className="size-2" /> {kind.label}
          </span>
          <span className="max-w-full truncate font-mono text-[12px] font-bold text-foreground/80">{m.key}</span>
        </div>
        {!editing && (
          <>
            <h2 className="mt-2.5 text-[26px] leading-[1.15]">{m.title}</h2>
            {m.tags.length > 0 && (
              <div className="mt-3 flex flex-wrap gap-1.5">
                {m.tags.map((t) => (
                  <LabelChip key={t}>{t}</LabelChip>
                ))}
              </div>
            )}
            <div className="mt-4 flex flex-wrap gap-2">
              <Button variant="outline" onClick={() => setBase(m)} disabled={busy}>
                <Pencil /> Edit
              </Button>
              <Button variant="ghost" className="text-destructive hover:text-destructive" onClick={() => setConfirm(true)} disabled={busy}>
                <Trash2 /> Forget
              </Button>
            </div>
          </>
        )}
      </header>

      {base ? (
        <MemoryEditForm
          m={base}
          potKey={potKey}
          save={(args) => call("memoryRemember", args)}
          latest={m}
          onReload={async () => {
            await q.refetch();
            setBase(null);
          }}
          onDone={() => setBase(null)}
        />
      ) : (
        <div className="flex-1 space-y-7 px-6 py-5">
          <section aria-label="Body">
            <Markdown>{m.body}</Markdown>
          </section>
          <section aria-labelledby="mem-meta">
            <SectionLabel>
              <span id="mem-meta">Details</span>
            </SectionLabel>
            <dl className="mt-2.5 grid grid-cols-[6.5rem_minmax(0,1fr)] gap-x-3 gap-y-2.5 text-[13px]">
              <dt className="text-muted-foreground">Kind</dt>
              <dd className="flex items-center gap-1.5">
                <KindDot kind={m.kind} className="size-2" /> {kind.label}
                <span className="text-muted-foreground">· {kind.hint}</span>
              </dd>
              {m.task && (
                <>
                  <dt className="text-muted-foreground">Learned on</dt>
                  <dd>
                    <button
                      type="button"
                      onClick={() => openTask(m.task!)}
                      className="inline-flex items-center gap-1 rounded font-mono text-[12px] font-bold underline-offset-2 hover:underline focus-visible:underline"
                    >
                      <Link2 className="size-3" aria-hidden="true" /> {m.task}
                    </button>
                  </dd>
                </>
              )}
              <dt className="text-muted-foreground">Created</dt>
              <dd className="flex flex-wrap items-center gap-1.5">
                {m.created_by && <Avatar name={m.created_by} className="size-5 text-[10px]" />}
                {m.created_by ?? "unknown"} <span className="text-muted-foreground">· {fmtAgo(m.created_at, now)}</span>
              </dd>
              <dt className="text-muted-foreground">Last updated</dt>
              <dd className="flex flex-wrap items-center gap-1.5">
                {m.updated_by && <Avatar name={m.updated_by} className="size-5 text-[10px]" />}
                {m.updated_by ?? "unknown"} <span className="text-muted-foreground">· {fmtAgo(m.updated_at, now)}</span>
              </dd>
              <dt className="text-muted-foreground">Revision</dt>
              <dd className="font-mono text-[12px]">r{m.revision}</dd>
            </dl>
          </section>
        </div>
      )}

      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Forget this memory?</AlertDialogTitle>
            <AlertDialogDescription>
              “{m.title}” leaves the pantry for every cook and person on {potKey}. Its earlier revisions go with it. If it is only out of
              date, edit it instead. Cooks see the removal with their next task.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <label className="flex items-start gap-2 text-[13px]">
            <input type="checkbox" checked={notifyForget} onChange={(e) => setNotifyForget(e.target.checked)} className="mt-0.5 size-4 accent-[var(--primary)]" />
            <span>
              Tell everyone now
              <span className="block text-[12px] text-muted-foreground">Wakes every cook&rsquo;s sessions with the removal on the pass.</span>
            </span>
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(e) => {
                e.preventDefault();
                void forget();
              }}
            >
              {busy && <Loader2 className="animate-spin" />} Forget it
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

type RememberArgs = MemoryRememberArgs;

function isConflict(e: unknown) {
  return errorStatus(e) === 409 || /revision|saved it meanwhile/i.test(errorMessage(e));
}

function MemoryEditForm({
  m,
  latest,
  potKey,
  save,
  onReload,
  onDone,
}: {
  m: MemoryView;
  /** The live copy: differs from `m` when somebody saved while this form was open. */
  latest: MemoryView;
  potKey: string;
  save: (a: RememberArgs) => Promise<unknown>;
  onReload: () => Promise<void>;
  onDone: () => void;
}) {
  const tagOptions = useTagOptions(potKey);
  const [title, setTitle] = useState(m.title);
  const [body, setBody] = useState(m.body);
  const [kind, setKind] = useState(m.kind);
  const [tags, setTags] = useState<string[]>(m.tags);
  const [busy, setBusy] = useState(false);
  const [conflict, setConflict] = useState(false);
  const stale = latest.revision !== m.revision || conflict;
  const dirty = title !== m.title || body !== m.body || kind !== m.kind || tags.join() !== m.tags.join();

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await save({ project: potKey, key: m.key, title: title.trim(), body, kind, tags: tags.join(","), revision: m.revision });
      toast.success("Saved", { description: title.trim() });
      onDone();
    } catch (err) {
      if (isConflict(err)) setConflict(true);
      else toast.error("Could not save", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex-1 space-y-4 px-6 py-5">
      {stale && (
        <div role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3.5 text-[13px]">
          <p className="font-medium">Someone saved this memory while you were editing.</p>
          <p className="mt-1 text-muted-foreground">
            You started from r{m.revision}; the pantry now holds r{latest.revision}
            {latest.updated_by ? ` (by ${latest.updated_by})` : ""}. Saving yours would overwrite theirs, so it is held back.
          </p>
          <Button type="button" variant="outline" size="sm" className="mt-2.5" onClick={() => void onReload()}>
            Reload their version
          </Button>
        </div>
      )}
      <div className="space-y-1.5">
        <Label htmlFor="m-title">Title</Label>
        <Input id="m-title" required maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="m-body">Body (markdown)</Label>
        <Textarea id="m-body" required rows={12} value={body} onChange={(e) => setBody(e.target.value)} className="font-mono text-[13px]" />
      </div>
      <div className="grid gap-4 sm:grid-cols-[1fr_10rem]">
        <div className="space-y-1.5">
          <Label>Tags</Label>
          <TagInput label="Tags" value={tags} onChange={setTags} normalize={(s) => s.toLowerCase()} placeholder="add a tag" {...tagOptions} />
        </div>
        <KindSelect id="m-kind" value={kind} onChange={setKind} />
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onDone}>
          {dirty ? "Discard" : "Close"}
        </Button>
        <Button type="submit" disabled={busy || stale || !title.trim() || !body.trim()}>
          {busy && <Loader2 className="animate-spin" />} Save
        </Button>
      </div>
    </form>
  );
}

function KindSelect({ id, value, onChange }: { id: string; value: string; onChange: (v: string) => void }) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>Kind</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {KINDS.map((k) => (
            <SelectItem key={k.id} value={k.id}>
              <KindDot kind={k.id} className="size-2" /> {k.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

// ---------------------------------------------------------------- new memory

export function NewMemoryDialog({
  potKey,
  open,
  onClose,
  onSaved,
}: {
  potKey: string;
  open: boolean;
  onClose: () => void;
  onSaved: (key: string) => void;
}) {
  const call = useProcCall();
  const tagOptions = useTagOptions(potKey, open);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [kind, setKind] = useState("note");
  const [tags, setTags] = useState<string[]>([]);
  const [key, setKey] = useState("");
  const [notify, setNotify] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setTitle("");
      setBody("");
      setKind("note");
      setTags([]);
      setKey("");
    }
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!title.trim() || !body.trim()) return;
    setBusy(true);
    try {
      const m = await call("memoryRemember", {
        project: potKey,
        title: title.trim(),
        body,
        kind,
        tags: tags.length ? tags.join(",") : null,
        key: key.trim() || null,
        // a new memory must not exist yet: saving over an existing key conflicts instead of overwriting it
        revision: 0,
        notify: notify || null,
        station: null,
      });
      toast.success("On the shelf", { description: m.title });
      onSaved(m.key);
      onClose();
    } catch (err) {
      toast.error(isConflict(err) ? "That key is already in the pantry" : "Could not save", {
        description: isConflict(err) ? "Pick another key, or open the existing memory and edit it." : errorMessage(err),
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Stock the pantry</DialogTitle>
          <DialogDescription>
            Something the next cook or teammate on {potKey} should know: a convention, a gotcha, a decision and its reason, how to run
            things. Never secrets.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="nm-title">Title</Label>
            <Input id="nm-title" required maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Run the integration tests with just e2e" autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="nm-body">Body (markdown)</Label>
            <Textarea id="nm-body" required rows={7} value={body} onChange={(e) => setBody(e.target.value)} placeholder="What you learned: commands, file paths, and the why." />
          </div>
          <div className="grid gap-4 sm:grid-cols-[1fr_10rem]">
            <div className="space-y-1.5">
              <Label>Tags</Label>
              <TagInput label="Tags" value={tags} onChange={setTags} normalize={(s) => s.toLowerCase()} placeholder="add a tag" {...tagOptions} />
            </div>
            <KindSelect id="nm-kind" value={kind} onChange={setKind} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="nm-key">Key (optional)</Label>
            <Input
              id="nm-key"
              value={key}
              onChange={(e) => setKey(e.target.value.toLowerCase())}
              placeholder="derived from the title"
              pattern="[a-z0-9._\-]*"
              className="font-mono text-[13px]"
            />
            <p className="text-[12px] text-muted-foreground">Lowercase letters, digits, dots, dashes and underscores. Stays the same across edits.</p>
          </div>
          <label className="flex items-start gap-2 text-[13px]">
            <input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} className="mt-0.5 size-4 accent-[var(--primary)]" />
            <span>
              Tell everyone now
              <span className="block text-[12px] text-muted-foreground">
                Wakes every cook&rsquo;s sessions with it on the pass. Otherwise cooks get it with their next task.
              </span>
            </span>
          </label>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !title.trim() || !body.trim()}>
              {busy && <Loader2 className="animate-spin" />} Put it on the shelf
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
