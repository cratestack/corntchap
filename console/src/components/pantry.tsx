import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useLocation, useSearchParams } from "react-router";
import type { MemoryRememberArgs, MemoryView } from "@corntchap/api";
import { ChevronDown, ChevronRight, Link2, Loader2, Pencil, Plus, Search, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProc, useProcCall } from "@/lib/data";
import { useProcInfinite } from "@/lib/paging";
import { errorMessage, errorStatus } from "@/lib/api";
import { fmtAgo, plural, useNow } from "@/lib/format";
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
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Avatar, EmptyState, LabelChip, RowSkeleton, SectionLabel } from "./bits";
import { RefreshButton } from "./refresh-button";
import { Markdown } from "./markdown";
import { TagInput } from "./tag-input";

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

const LIMIT = 100;
/** Memories per page of the unfiltered pantry. */
const PAGE = 100;

export function PantryView({ potKey, onNew }: { potKey: string; onNew: () => void }) {
  const [params, setParams] = useSearchParams();
  const loc = useLocation();
  const q = params.get("q") ?? "";
  const kind = params.get("kind") ?? "";
  const tags = useMemo(() => (params.get("tag") ?? "").split(",").filter(Boolean), [params]);
  const selected = params.get("memory");
  const filtered = !!(q.trim() || kind || tags.length);

  const [text, setText] = useState(q);
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => setText(q), [q]);
  useEffect(() => {
    if ((loc.state as { focus?: boolean } | null)?.focus) searchRef.current?.focus();
  }, [loc.state]);

  const setParam = (k: string, v: string | undefined) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );

  // debounce the search box into the URL
  useEffect(() => {
    if (text === q) return;
    const t = setTimeout(() => setParam("q", text.trim() ? text : undefined), 280);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

  const clear = () => {
    setText("");
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        for (const k of ["q", "kind", "tag"]) n.delete(k);
        return n;
      },
      { replace: true },
    );
  };

  // The unfiltered pantry (newest change first, a page at a time) feeds the list, the tag list and the "is it empty at all" check;
  // the search runs only with a filter.
  const all = useProcInfinite("memoryPage", { project: potKey, kind: null, tags: null }, { getCursor: (p) => p.nextCursor, pageSize: PAGE });
  const res = useProc(
    "memoryRecall",
    { project: potKey, query: q.trim() || null, kind: kind || null, tags: tags.length ? tags.join(",") : null, limit: LIMIT },
    { enabled: filtered },
  );
  const rows = filtered ? (res.data ?? []) : all.items;
  const tagCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of all.items) for (const t of r.tags) m.set(t, (m.get(t) ?? 0) + 1);
    for (const t of tags) if (!m.has(t)) m.set(t, 0);
    return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  }, [all.items, tags]);

  const openMemory = (key: string) => setParam("memory", key);
  const closeSheet = () =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.delete("memory");
        return n;
      },
      { replace: false },
    );

  const empty = !all.isLoading && !all.error && all.items.length === 0;

  return (
    <div className="max-w-5xl">
      <div role="search" aria-label="Search the pantry" className="space-y-2.5">
        <div className="relative min-w-0 sm:max-w-md">
          <Search className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            ref={searchRef}
            type="search"
            aria-label="Search the pantry"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Search keys, titles and bodies…"
            className="h-11 rounded-full pr-3 pl-10 text-[14px] md:h-9 md:text-[13px]"
          />
        </div>
        <div className="scrollbar-none -mx-1 flex items-center gap-2 overflow-x-auto px-1 pb-0.5" role="group" aria-label="Kinds">
          <KindChip active={!kind} onClick={() => setParam("kind", undefined)} count={all.items.length}>
            All
          </KindChip>
          {KINDS.map((k) => (
            <KindChip
              key={k.id}
              active={kind === k.id}
              onClick={() => setParam("kind", kind === k.id ? undefined : k.id)}
              count={all.items.filter((r) => r.kind === k.id).length}
            >
              <KindDot kind={k.id} className="size-2" /> {k.label}
            </KindChip>
          ))}
          <TagFilter
            tags={tagCounts}
            selected={tags}
            onChange={(next) => setParam("tag", next.length ? next.join(",") : undefined)}
          />
          {filtered && (
            <button type="button" onClick={clear} className="shrink-0 px-1 text-[12.5px] whitespace-nowrap text-primary hover:underline">
              Clear
            </button>
          )}
        </div>
      </div>

      <div className="mt-4 space-y-2">
        {all.isLoading && <RowSkeleton rows={5} />}
        {!!all.error && !all.isLoading && (
          <EmptyState band={false} title="The pantry would not open">
            {errorMessage(all.error)}
          </EmptyState>
        )}
        {empty && (
          <EmptyState title="Nothing in the pantry yet" action={<Button onClick={onNew}>Stock the pantry</Button>}>
            Agents stock it as they learn the codebase: conventions, gotchas, how to run things. The next cook reads it before
            starting, so nobody works out the same thing twice. You can put the first jar on the shelf yourself.
          </EmptyState>
        )}
        {filtered && !res.isLoading && !res.error && !empty && rows.length === 0 && (
          <EmptyState
            band={false}
            title="Nothing on the shelf matches that"
            action={
              <Button variant="outline" onClick={clear}>
                Clear filters
              </Button>
            }
          >
            Try fewer words, or loosen a kind or tag. Search looks at keys, titles and bodies.
          </EmptyState>
        )}
        {filtered && !!res.error && <EmptyState band={false} title="That search failed">{errorMessage(res.error)}</EmptyState>}
        {filtered && res.isLoading && <RowSkeleton rows={3} />}
        {rows.length > 0 && (
          <>
            <p className="label-caps px-0.5" aria-live="polite">
              {plural(rows.length, "memory", "memories")}
              {filtered && rows.length >= LIMIT && ` (showing the first ${LIMIT})`}
              {filtered && ` · ${all.items.length}${all.hasNextPage ? "+" : ""} in the pantry`}
            </p>
            <ul className="grid grid-cols-[minmax(0,1fr)] gap-2.5 max-md:pb-16 lg:grid-cols-2">
              {rows.map((m) => (
                <li key={m.key}>
                  <MemoryRow m={m} selected={selected === m.key} onOpen={openMemory} />
                </li>
              ))}
            </ul>
            {!filtered && all.hasNextPage && (
              <div className="flex justify-center pt-1">
                <Button type="button" variant="outline" disabled={all.isFetchingNextPage} onClick={() => void all.fetchNextPage()}>
                  {all.isFetchingNextPage && <Loader2 className="animate-spin" />}
                  Show more memories
                </Button>
              </div>
            )}
          </>
        )}
      </div>

      <Button
        type="button"
        onClick={onNew}
        aria-label="Stock the pantry"
        className="bottom-tabbar fixed right-4 z-30 mb-4 h-12 rounded-full px-5 shadow-lg md:hidden"
      >
        <Plus /> Stock
      </Button>

      <MemorySheet potKey={potKey} memoryKey={selected} onClose={closeSheet} />
    </div>
  );
}

function KindChip({ active, count, onClick, children }: { active: boolean; count: number; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full border px-3.5 text-[13px] whitespace-nowrap transition-colors md:h-8 md:px-3 md:text-[12.5px]",
        active ? "border-primary/60 bg-primary/10 font-medium text-foreground" : "border-input bg-card text-muted-foreground hover:border-foreground/30",
      )}
    >
      {children}
      <span className="font-mono text-[10.5px] text-muted-foreground">{count}</span>
    </button>
  );
}

function TagFilter({
  tags,
  selected,
  onChange,
}: {
  tags: [string, number][];
  selected: string[];
  onChange: (t: string[]) => void;
}) {
  const value = selected.join(", ");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={tags.length === 0}
        className={cn(
          "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full border px-3.5 text-[13px] whitespace-nowrap transition-colors disabled:opacity-50 md:h-8 md:px-3 md:text-[12.5px]",
          value ? "border-primary/60 bg-primary/10" : "border-input bg-card hover:border-foreground/30",
        )}
      >
        <span className="text-muted-foreground">Tag</span>
        {value && <span className="max-w-40 truncate font-medium">{value}</span>}
        <ChevronDown className="size-3 text-muted-foreground" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-80 min-w-48 overflow-y-auto">
        <DropdownMenuLabel className="label-caps py-1">Memories with all of</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {tags.map(([t, n]) => (
          <DropdownMenuCheckboxItem
            key={t}
            checked={selected.includes(t)}
            onSelect={(e) => e.preventDefault()}
            onCheckedChange={(on) => onChange(on ? [...selected, t] : selected.filter((x) => x !== t))}
          >
            <span className="min-w-0 flex-1 truncate">{t}</span>
            <span className="font-mono text-[11px] text-muted-foreground">{n}</span>
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

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
        "group block w-full min-w-0 rounded-[14px] border bg-card px-3.5 py-3 text-left transition-colors active:bg-muted/60 md:px-4",
        "hover:border-foreground/25 focus-visible:border-foreground/40",
        selected ? "border-primary ring-1 ring-primary/40" : "border-border",
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
          <TagInput label="Tags" value={tags} onChange={setTags} normalize={(s) => s.toLowerCase()} placeholder="add a tag" />
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
              <TagInput label="Tags" value={tags} onChange={setTags} normalize={(s) => s.toLowerCase()} placeholder="add a tag" />
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
