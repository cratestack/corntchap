import { useEffect, useMemo, useState } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";
import type { MenuView, ProjectSummary, TaskView, ViewOut } from "@corntchap/api";
import { ChevronDown, Loader2, Pencil, Plus, Settings2, SlidersHorizontal, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useProjects } from "@/lib/data";
import { useBoardData, type BucketMore } from "@/lib/board";
import { useProcInfinite } from "@/lib/paging";
import { useLive } from "@/lib/live";
import { useSession } from "@/lib/session";
import { useUi } from "@/lib/ui";
import { useUrlFilters } from "@/lib/url-filters";
import { useDebounced } from "@/lib/use-debounced";
import { QUERY_DEBOUNCE_MS } from "@/lib/list-query";
import { useStoreValue } from "@/lib/use-store-value";
import { matchText, useStaticSource, type ListSource } from "@/lib/listbox";
import { useEventsSource, useSavedViewsSource } from "@/lib/sources";
import { BUCKETS, bucketOf, type Bucket } from "@/lib/format";
import { forgetView, isBaseView, lastPot, lastView, rememberPot, rememberView, type BaseView, type Filters } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
import { useIsPhone } from "@/lib/media";
import { errorMessage } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { VirtualMenu } from "@/components/ui/virtual-menu";
import { BUCKET_DOT, EmptyState, RowSkeleton } from "@/components/bits";
import { NoPot } from "@/components/no-pot";
import { PaletteButton } from "@/components/shell";
import { RefreshButton } from "@/components/refresh-button";
import { EventFeed } from "@/components/event-feed";
import { FilterBar, type FacetDef, type FacetOption } from "@/components/filter-bar";
import { TASK_FILTERS, useActorFacet, useTaskFacets } from "@/components/pot-filters";
import { TaskGroup, groupTasks } from "@/components/task-views";
import { BoardView, useColumns } from "@/components/board";
import { TaskSheet } from "@/components/task-sheet";
import { NewMemoryDialog, PantryView } from "@/components/pantry";
import { MenuDialog, MenuStrip, MenusView, useMenu, useMenuNames } from "@/components/menus";
import { FilesView } from "@/components/files";
import { PassView } from "@/components/pass";
import { AskQuestionDialog, QuestionsView } from "@/components/questions";
import { DeleteViewDialog, ViewDialog, layoutOf, useSavedView } from "@/components/views";
import { PotOptions } from "@/components/pot-options";

/** Tabs before the saved views (just the Board) and after them. */
const HEAD_VIEWS: { id: BaseView; label: string }[] = [{ id: "board", label: "Board" }];
const TAIL_VIEWS: { id: BaseView; label: string }[] = [
  { id: "activity", label: "Activity" },
  { id: "pantry", label: "Pantry" },
  { id: "menus", label: "Menus" },
  { id: "questions", label: "Questions" },
  { id: "pass", label: "Pass" },
  { id: "files", label: "Files" },
];

/** The task filters in the URL as the `Filters` the views read (only what is set). */
function toFilters(values: Record<keyof typeof TASK_FILTERS, string>, q: string): Filters {
  const f: Filters = {};
  for (const k of ["status", "label", "repo", "holder", "owner", "menu"] as const) if (values[k]) f[k] = values[k];
  if (q) f.q = q;
  return f;
}

function applyFilters(tasks: TaskView[], f: Filters, me: string): TaskView[] {
  const q = f.q?.trim().toLowerCase();
  return tasks.filter((t) => {
    if (f.status && bucketOf(t) !== f.status) return false;
    if (f.label && !t.labels.includes(f.label)) return false;
    if (f.repo && !t.repos.includes(f.repo)) return false;
    if (f.holder && t.claimed_by !== f.holder) return false;
    if (f.owner && (f.owner === "-" ? !!t.owner : t.owner !== (f.owner === "me" ? me : f.owner))) return false;
    if (f.menu && (f.menu === "-" ? !!t.menu : t.menu !== f.menu)) return false;
    if (q && !`${t.id} ${t.title}`.toLowerCase().includes(q)) return false;
    return true;
  });
}

export function PotRedirect() {
  const projects = useProjects();
  if (projects.isLoading) {
    return (
      <div className="px-8 py-10">
        <RowSkeleton />
      </div>
    );
  }
  const remembered = lastPot();
  const first = projects.data?.find((p) => p.key === remembered) ?? projects.data?.[0];
  if (first) return <Navigate to={`/pots/${first.key}/${lastView(first.key)}`} replace />;
  return <NoPot />;
}

export function KitchenPage() {
  const { key = "", view, viewKey } = useParams();
  const potKey = key.toUpperCase();
  const base: BaseView = isBaseView(view) ? view : "board";
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const openTask = useOpenTask();
  const { openNewTask } = useUi();
  const projects = useProjects();
  const project = projects.data?.find((p) => p.key === potKey);
  const saved = useSavedView(potKey, viewKey, !!project);
  const savedView = saved.view;
  const [viewDialog, setViewDialog] = useState<{ open: boolean; view: ViewOut | null }>({ open: false, view: null });
  const [deleting, setDeleting] = useState<ViewOut | null>(null);
  const taskView = !!viewKey || base === "board";
  const { events } = useLive();
  const url = useUrlFilters(TASK_FILTERS);
  const filters = useMemo(() => toFilters(url.values, url.q), [url.values, url.q]);
  // the board asks the server with the text once it has rested
  const restedQ = useDebounced(url.q, QUERY_DEBOUNCE_MS);
  const board = useBoardData(potKey, { ...filters, q: url.q ? restedQ : "" }, !!project && !viewKey && base === "board");
  const selected = params.get("task");
  const [newMemory, setNewMemory] = useState(false);
  const [asking, setAsking] = useState(false);
  const openQuestions = project?.open_questions ?? 0;
  const [menuDialog, setMenuDialog] = useState<{ open: boolean; menu: MenuView | null }>({ open: false, menu: null });
  const selectedMenu = useMenu(potKey, filters.menu).menu;
  const menuNames = useMenuNames(potKey, !!project);
  const facets = useTaskFacets(project, selectedMenu);

  useEffect(() => {
    if (project && viewKey) {
      if (savedView) rememberView(potKey, `v/${viewKey}`);
    } else if (project) rememberView(potKey, base);
    if (project) rememberPot(potKey);
    document.title = project ? `${project.name}${savedView ? ` · ${savedView.name}` : ""} · corntchap` : "corntchap";
  }, [potKey, base, project, viewKey, savedView]);

  // `?new=1` (from the command palette) opens the New menu dialog on the Menus tab
  useEffect(() => {
    if (base === "menus" && params.get("new")) {
      setMenuDialog({ open: true, menu: null });
      setParams((p) => { const n = new URLSearchParams(p); n.delete("new"); return n; }, { replace: true });
    }
  }, [base, params, setParams]);

  const setFilter = (k: keyof Filters, v: string | undefined) => (k === "q" ? url.setQ(v ?? "") : url.set({ [k]: v ?? "" }));
  const clearFilters = url.clear;
  const closeSheet = () =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.delete("task");
        return n;
      },
      { replace: false },
    );

  if (projects.isLoading) {
    return (
      <div className="px-4 py-6 md:px-5 md:py-8 lg:px-8">
        <RowSkeleton />
      </div>
    );
  }
  if (!project && (projects.data ?? []).length === 0 && !projects.error) return <NoPot />;
  if (!project) {
    return (
      <div className="mx-auto max-w-2xl px-4 py-10 sm:px-5 sm:py-16">
        <EmptyState title={`No pot called ${potKey}`} action={<Button asChild variant="outline"><Link to="/">Back to the kitchen</Link></Button>}>
          Either it does not exist or you are not on its team.
        </EmptyState>
      </div>
    );
  }

  const potEvents = events.filter((e) => e.project === potKey);
  // while one menu is selected every row is on it, so the chip would only repeat the strip
  const chipNames = filters.menu ? {} : menuNames;

  return (
    <div className="px-4 pt-3 pb-6 md:px-5 md:pt-6 md:pb-16 lg:px-8">
      <PotHeader
        project={project}
        base={base}
        viewKey={viewKey}
        savedName={savedView?.name}
        taskView={taskView}
        onNewView={() => setViewDialog({ open: true, view: null })}
        filters={filters}
        setFilter={setFilter}
        clearFilters={clearFilters}
        facets={facets}
        q={url.q}
        setQ={url.setQ}
        onFacets={(patch) => url.set(patch as never)}
        onNew={() =>
          base === "pantry"
            ? setNewMemory(true)
            : base === "menus"
              ? setMenuDialog({ open: true, menu: null })
              : base === "questions"
                ? setAsking(true)
                : openNewTask(potKey, filters.menu)
        }
        openQuestions={openQuestions}
      />

      {savedView && (
        <ViewBar
          view={savedView}
          onEdit={() => setViewDialog({ open: true, view: savedView })}
          onDelete={() => setDeleting(savedView)}
        />
      )}

      {selectedMenu && taskView && <MenuStrip menu={selectedMenu} />}

      <div className="mt-3 md:mt-6">
        {viewKey ? (
          <SavedViewBody
            potKey={potKey}
            view={savedView}
            loadingViews={saved.isLoading}
            filters={filters}
            selected={selected}
            onOpen={openTask}
            events={potEvents}
            onClear={clearFilters}
            menus={chipNames}
          />
        ) : null}
        {!viewKey && base === "board" && <BoardView loading={board.loading} columns={board} selected={selected} onOpen={openTask} onNew={() => openNewTask(potKey, filters.menu)} bare={board.bare} menus={chipNames} />}
        {!viewKey && base === "activity" && <ActivityView project={project} />}
        {!viewKey && base === "pantry" && <PantryView potKey={potKey} onNew={() => setNewMemory(true)} />}
        {!viewKey && base === "files" && <FilesView potKey={potKey} />}
        {!viewKey && base === "pass" && <PassView potKey={potKey} />}
        {!viewKey && base === "questions" && <QuestionsView potKey={potKey} onAsk={() => setAsking(true)} />}
        {!viewKey && base === "menus" && (
          <MenusView
            potKey={potKey}
            onNew={() => setMenuDialog({ open: true, menu: null })}
            onEdit={(m) => setMenuDialog({ open: true, menu: m })}
          />
        )}
      </div>

      <TaskSheet id={selected} onClose={closeSheet} />
      <ViewDialog
        potKey={potKey}
        open={viewDialog.open}
        view={viewDialog.view}
        onClose={() => setViewDialog((s) => ({ ...s, open: false }))}
        onSaved={(v) => void navigate(`/pots/${potKey}/v/${v.key}`)}
      />
      <DeleteViewDialog
        potKey={potKey}
        view={deleting}
        onClose={() => setDeleting(null)}
        onDeleted={(v) => {
          forgetView(potKey, v.key);
          if (v.key === viewKey) void navigate(`/pots/${potKey}/board`, { replace: true });
        }}
      />
      <MenuDialog
        potKey={potKey}
        open={menuDialog.open}
        menu={menuDialog.menu}
        onClose={() => setMenuDialog((s) => ({ ...s, open: false }))}
      />
      <AskQuestionDialog potKey={potKey} open={asking} onClose={() => setAsking(false)} />
      <NewMemoryDialog
        potKey={potKey}
        open={newMemory}
        onClose={() => setNewMemory(false)}
        onSaved={(k) => setParams((p) => { const n = new URLSearchParams(p); n.set("memory", k); return n; }, { replace: false })}
      />
    </div>
  );
}

// ---------------------------------------------------------------- header

const UNREAD_BADGE =
  "relative ml-1.5 inline-flex min-w-4 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10.5px] leading-4 font-bold text-primary-foreground";

const ALL_VIEWS = [...HEAD_VIEWS, ...TAIL_VIEWS];

/** Counts per status as tappable chips: a tap filters the board to that status, a second tap clears it. */
function StatusChips({
  project,
  status,
  onPick,
  withLabels,
}: {
  project: ProjectSummary;
  status: string | undefined;
  onPick: (b: Bucket | undefined) => void;
  withLabels: boolean;
}) {
  const items: { id: Bucket; n: number; short: string }[] = [
    { id: "ready", n: project.ready, short: "ready to scoop" },
    { id: "claimed", n: project.claimed, short: "on the fire" },
    { id: "waiting", n: project.blocked, short: "waiting" },
    { id: "done", n: project.done, short: "served" },
    { id: "cancelled", n: project.tossed, short: "tossed" },
  ];
  return (
    <div className={cn("flex flex-wrap items-center", withLabels ? "gap-1.5" : "gap-1")} role="group" aria-label="Filter by status">
      {items
        .filter((i) => i.id !== "cancelled" || i.n > 0)
        .map((i) => {
          const on = status === i.id;
          return (
            <button
              key={i.id}
              type="button"
              aria-pressed={on}
              aria-label={`${i.n} ${i.short}`}
              title={`${i.n} ${i.short}`}
              onClick={() => onPick(on ? undefined : i.id)}
              className={cn(
                "inline-flex items-center gap-1.5 rounded-full border text-[12.5px] transition-colors",
                withLabels ? "h-8 px-3" : "h-10 px-2.5",
                on ? "border-primary/60 bg-primary/10 font-medium" : "border-border bg-card text-muted-foreground hover:border-foreground/30",
              )}
            >
              <span className={cn("size-2 rounded-full", BUCKET_DOT[i.id], i.id === "claimed" && i.n > 0 && "live-dot")} aria-hidden="true" />
              <b className="font-mono text-[12px] font-bold text-foreground tabular-nums">{i.n}</b>
              {withLabels && <span>{i.short}</span>}
            </button>
          );
        })}
    </div>
  );
}

interface HeaderProps {
  project: ProjectSummary;
  base: BaseView;
  viewKey: string | undefined;
  /** The name of the saved view that is open, once known. */
  savedName: string | undefined;
  taskView: boolean;
  onNewView: () => void;
  filters: Filters;
  setFilter: (k: keyof Filters, v: string | undefined) => void;
  clearFilters: () => void;
  /** The task filters' chips: their options are read from the pot a page at a time. */
  facets: FacetDef[];
  q: string;
  setQ: (q: string) => void;
  onFacets: (patch: Record<string, string | string[]>) => void;
  onNew: () => void;
  openQuestions: number;
}

function PotHeader(props: HeaderProps) {
  return useIsPhone() ? <PhoneHeader {...props} /> : <WideHeader {...props} />;
}

/** The facets' values as the filter bar reads them. */
const facetValues = (f: Filters): Record<string, string | undefined> => ({ menu: f.menu, status: f.status, label: f.label, repo: f.repo, owner: f.owner, holder: f.holder });

// ---------------------------------------------------------------- saved views menu

/** A row of the views menu: the board, the "new view" action, or one saved view. */
type ViewRow = { kind: "board" } | { kind: "new" } | { kind: "view"; view: ViewOut };

/**
 * A pot's saved views as a searchable, paged menu (they may be thousands): on a phone the Board and "New view" are
 * in it too, so it is the whole views switcher; on wide screens the Board is a tab beside it.
 */
function ViewsMenu({ potKey, viewKey, savedName, onNewView, phone }: { potKey: string; viewKey: string | undefined; savedName: string | undefined; onNewView: () => void; phone: boolean }) {
  const navigate = useNavigate();
  const q = useStoreValue("");
  const views = useSavedViewsSource(potKey, q.value);
  const source = useMemo<ListSource<ViewRow>>(() => {
    const lead: ViewRow[] = phone ? [...(matchText(q.value, "Board") ? [{ kind: "board" as const }] : []), ...(matchText(q.value, "New view") ? [{ kind: "new" as const }] : [])] : [];
    return { ...views, items: [...lead, ...views.items.map((view): ViewRow => ({ kind: "view", view }))] };
  }, [views, phone, q.value]);
  const current = viewKey ? (savedName ?? "View") : "Board";
  const trigger = phone ? (
    <button type="button" className="inline-flex h-10 max-w-32 shrink-0 items-center gap-1 rounded-full border border-border bg-card pr-2.5 pl-3.5 text-[13px] font-medium active:bg-muted">
      <span className="truncate">{current}</span>
      <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  ) : (
    <button
      type="button"
      aria-current={viewKey ? "page" : undefined}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-lg px-3.5 py-1.5 text-[13px] font-medium whitespace-nowrap transition-colors hover:text-foreground pointer-coarse:py-2.5",
        viewKey ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
      )}
    >
      <span className="max-w-40 truncate">{viewKey ? current : "Views"}</span>
      <ChevronDown className="size-3 text-muted-foreground" aria-hidden="true" />
    </button>
  );
  return (
    <VirtualMenu<ViewRow>
      label="Views"
      mode="single"
      source={source}
      onQuery={q.set}
      itemKey={(r) => (r.kind === "view" ? r.view.key : r.kind)}
      selected={(r) => (r.kind === "view" ? r.view.key === viewKey : r.kind === "board" && !viewKey)}
      onSelect={(r) => (r.kind === "new" ? onNewView() : void navigate(r.kind === "board" ? `/pots/${potKey}/board` : `/pots/${potKey}/v/${r.view.key}`))}
      renderItem={(r) =>
        r.kind === "new" ? (
          <span className="flex items-center gap-2">
            <Plus className="size-4" aria-hidden="true" /> New view
          </span>
        ) : r.kind === "board" ? (
          <span>Board</span>
        ) : (
          <span className="block truncate">{r.view.name}</span>
        )
      }
      empty="No saved views yet."
      align="start"
      trigger={trigger}
    />
  );
}

function PhoneHeader({ project, base, viewKey, savedName, taskView, onNewView, filters, setFilter, clearFilters, facets, q, setQ, onFacets, onNew }: HeaderProps) {
  const [sheet, setSheet] = useState(false);
  const activeCount = Object.values(filters).filter(Boolean).length;

  if (!taskView) {
    const label = ALL_VIEWS.find((v) => v.id === base)?.label ?? "Pot";
    // the pantry has its own floating "Stock" button on phones
    const action = base === "menus" ? "New menu" : base === "questions" ? "Ask" : null;
    return (
      <header className="flex h-10 items-center justify-between gap-3">
        <h1 className="text-[22px] leading-none">{label}</h1>
        {action && (
          <Button size="sm" variant="outline" onClick={onNew}>
            <Plus /> {action}
          </Button>
        )}
      </header>
    );
  }

  return (
    <header className="flex flex-wrap items-center gap-2">
      <ViewsMenu potKey={project.key} viewKey={viewKey} savedName={savedName} onNewView={onNewView} phone />
      {/* the board has its own status switcher under this line; lists filter by status here */}
      <div className="order-last min-w-0 basis-full">
        {viewKey && <StatusChips project={project} status={filters.status} onPick={(b) => setFilter("status", b)} withLabels={false} />}
      </div>
      <button
        type="button"
        onClick={() => setSheet(true)}
        aria-label={activeCount ? `Display and filter, ${activeCount} active` : "Display and filter"}
        className={cn(
          "relative ml-auto inline-flex size-10 shrink-0 items-center justify-center rounded-full border active:bg-muted",
          activeCount ? "border-primary/60 bg-primary/10" : "border-border bg-card",
        )}
      >
        <SlidersHorizontal className="size-4" aria-hidden="true" />
        {activeCount > 0 && (
          <span className="absolute -top-1 -right-1 grid min-w-4 place-items-center rounded-full bg-primary px-1 font-mono text-[10px] leading-4 font-bold text-primary-foreground">{activeCount}</span>
        )}
      </button>
      <FilterSheet open={sheet} onOpenChange={setSheet} project={project} filters={filters} clearFilters={clearFilters} facets={facets} q={q} setQ={setQ} onFacets={onFacets} />
    </header>
  );
}

/** Phone: the text and every facet (menu, status, label, repository, owner, cook) in one bottom sheet (Linear's display options). */
function FilterSheet({
  open,
  onOpenChange,
  project,
  filters,
  clearFilters,
  facets,
  q,
  setQ,
  onFacets,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  project: ProjectSummary;
  filters: Filters;
  clearFilters: () => void;
  facets: FacetDef[];
  q: string;
  setQ: (q: string) => void;
  onFacets: (patch: Record<string, string | string[]>) => void;
}) {
  const active = Object.values(filters).some(Boolean);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-sm:flex max-sm:flex-col max-sm:gap-0 max-sm:p-0" onOpenAutoFocus={(e) => e.preventDefault()}>
        <DialogHeader className="flex-row items-center justify-between gap-2 px-5 pt-1 pb-3 text-left max-sm:pt-7">
          <div>
            <DialogTitle>Display and filter</DialogTitle>
            <DialogDescription className="sr-only">Narrow the {project.key} board by menu, status, label, repository, owner, cook or text.</DialogDescription>
          </div>
          <div className="mr-8 flex items-center gap-2">
            <RefreshButton />
            <PotOptions project={project} />
          </div>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto border-t border-border px-5 py-4">
          <FilterBar q={q} onQ={setQ} facets={facets} values={facetValues(filters)} onChange={onFacets} onClear={clearFilters} searchPlaceholder="Search id or title" />
        </div>
        <div className="flex gap-2 border-t border-border bg-card px-5 pt-3 pb-[calc(0.75rem+env(safe-area-inset-bottom))]">
          <Button variant="outline" className="flex-1" onClick={clearFilters} disabled={!active}>
            Clear
          </Button>
          <Button className="flex-1" onClick={() => onOpenChange(false)}>
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function WideHeader({ project, base, viewKey, savedName, taskView, onNewView, filters, clearFilters, setFilter, facets, q, setQ, onFacets, onNew, openQuestions }: HeaderProps) {
  const loc = useLocation();
  const active = Object.values(filters).some(Boolean);

  const tab = (v: { id: BaseView; label: string }) => (
    <Link
      key={v.id}
      to={{ pathname: `/pots/${project.key}/${v.id}`, search: v.id !== "board" || !taskView ? "" : loc.search }}
      aria-current={!viewKey && base === v.id && !(active && taskView) ? "page" : undefined}
      className={cn(
        "shrink-0 rounded-lg px-3.5 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground pointer-coarse:py-2.5",
        !viewKey && base === v.id && "bg-card text-foreground shadow-sm",
      )}
    >
      {v.label}
      {v.id === "pass" && project.unread_pass > 0 && (
        <span className={UNREAD_BADGE}>
          <span aria-hidden="true">{project.unread_pass}</span>
          <span className="sr-only"> unread {project.unread_pass === 1 ? "message" : "messages"}</span>
        </span>
      )}
      {v.id === "questions" && openQuestions > 0 && (
        <span className={UNREAD_BADGE}>
          <span aria-hidden="true">{openQuestions}</span>
          <span className="sr-only"> open {openQuestions === 1 ? "question" : "questions"}</span>
        </span>
      )}
    </Link>
  );

  return (
    <header>
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0">
          <p className="label-caps flex items-center gap-2">
            <Link to={`/pots/${project.key}/about`} title={`About ${project.name}`} className="rounded bg-corn px-1.5 py-px font-bold !text-corn-foreground hover:underline focus-visible:underline">{project.key}</Link>
            {project.team && <span>team {project.team}</span>}
          </p>
          <h1 className="mt-1.5 truncate text-[28px] leading-[1.1] lg:text-[34px]">{project.name}</h1>
          <div className="mt-2.5">
            <StatusChips project={project} status={filters.status} onPick={(b) => setFilter("status", b)} withLabels />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <PaletteButton className="sm:hidden lg:inline-flex" />
          <RefreshButton />
          <PotOptions project={project} />
          <Button onClick={onNew}>
            <Plus /> {base === "pantry" ? "Stock the pantry" : base === "menus" ? "New menu" : base === "questions" ? "Ask a question" : "New task"}
          </Button>
        </div>
      </div>

      <nav aria-label="Views" className="mt-5 flex w-fit max-w-full flex-wrap items-center gap-1 rounded-xl bg-muted p-1">
        {HEAD_VIEWS.map(tab)}
        <ViewsMenu potKey={project.key} viewKey={viewKey} savedName={savedName} onNewView={onNewView} phone={false} />
        <button
          type="button"
          onClick={onNewView}
          className="shrink-0 rounded-lg px-3 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground hover:text-foreground pointer-coarse:py-2.5"
        >
          + View
        </button>
        {TAIL_VIEWS.map(tab)}
      </nav>

      {taskView && (
        <FilterBar
          className="mt-3"
          q={q}
          onQ={setQ}
          facets={facets}
          values={facetValues(filters)}
          onChange={onFacets}
          onClear={clearFilters}
          searchPlaceholder="Filter tasks"
        />
      )}
    </header>
  );
}

function ViewBar({ view, onEdit, onDelete }: { view: ViewOut; onEdit: () => void; onDelete: () => void }) {
  return (
    <div className="mt-3 flex items-start gap-3 rounded-[14px] border border-border bg-card px-3.5 py-2.5 md:mt-4 md:px-4 md:py-3">
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
          <b className="font-semibold">{view.name}</b>
          <span className="text-muted-foreground">
            {view.layout === "list" ? "list" : "board"}
            {view.created_by ? ` · made by ${view.created_by}` : ""}
          </span>
        </p>
        {view.description && <p className="mt-0.5 text-[12.5px] text-muted-foreground">{view.description}</p>}
        <p className="mt-1 font-mono text-[12px] break-words text-muted-foreground">
          {view.where.trim() || "all tasks"}
          {view.order ? ` · order by ${view.order}` : ""}
        </p>
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={`Options for the ${view.name} view`}
          className="inline-flex size-10 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground md:size-8"
        >
          <Settings2 className="size-4" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onEdit}>
            <Pencil /> Edit
          </DropdownMenuItem>
          <DropdownMenuItem variant="destructive" onSelect={onDelete}>
            <Trash2 /> Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

function SavedViewBody({
  potKey,
  view,
  loadingViews,
  filters,
  selected,
  onOpen,
  events,
  onClear,
  menus,
}: {
  potKey: string;
  view: ViewOut | undefined;
  loadingViews: boolean;
  filters: Filters;
  selected: string | null;
  onOpen: (id: string) => void;
  events: ReturnType<typeof useLive>["events"];
  onClear: () => void;
  menus: Record<string, string>;
}) {
  const me = useSession();
  // a view is read a page at a time, as far as the reader scrolls; the chips filter what is loaded
  const run = useProcInfinite("viewPage", { project: potKey, key: view?.key ?? null, where: null, orderBy: null }, { getCursor: (p) => p.nextCursor, pageSize: 50, maxLimit: 200, maxPages: Infinity, enabled: !!view });
  const tasks = run.items;
  const visible = applyFilters(tasks, filters, me.name);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = run;
  const more = useMemo<BucketMore>(
    () => ({ has: !!hasNextPage, loading: isFetchingNextPage, load: () => void (hasNextPage && !isFetchingNextPage && fetchNextPage()) }),
    [hasNextPage, isFetchingNextPage, fetchNextPage],
  );
  const columns = useColumns(visible, true, more);
  if (loadingViews) return <RowSkeleton rows={4} />;
  if (!view) {
    return (
      <EmptyState title="No such view" action={<Button asChild variant="outline"><Link to={`/pots/${potKey}/board`}>Back to the board</Link></Button>}>
        It may have been deleted, or it was never made on this pot.
      </EmptyState>
    );
  }
  if (run.error && !run.isLoading) {
    return (
      <EmptyState band={false} title="This view would not run">
        <span className="font-mono text-[12.5px] break-words">{errorMessage(run.error)}</span>
      </EmptyState>
    );
  }
  if (!run.isLoading && tasks.length === 0) {
    return (
      <EmptyState title="No tasks match this view">
        <code className="font-mono text-[12.5px] break-words">{view.where.trim() || "(no condition)"}</code>
      </EmptyState>
    );
  }
  return layoutOf(view) === "list" ? (
    <ListView
      loading={run.isLoading}
      error={null}
      tasks={visible}
      total={tasks.length}
      filtered={Object.keys(filters).length > 0}
      selected={selected}
      onOpen={onOpen}
      events={events}
      potKey={potKey}
      onClear={onClear}
      onNew={() => undefined}
      menus={menus}
      keepOrder
      more={more}
    />
  ) : (
    <BoardView loading={run.isLoading} columns={columns} selected={selected} onOpen={onOpen} onNew={() => undefined} bare={tasks.length === 0} menus={menus} />
  );
}

// ---------------------------------------------------------------- views

function ListView({
  loading,
  error,
  tasks,
  total,
  filtered,
  selected,
  onOpen,
  events,
  potKey,
  onClear,
  onNew,
  menus,
  keepOrder = false,
  more,
}: {
  loading: boolean;
  error: unknown;
  tasks: TaskView[];
  total: number;
  filtered: boolean;
  selected: string | null;
  onOpen: (id: string) => void;
  events: ReturnType<typeof useLive>["events"];
  potKey: string;
  onClear: () => void;
  onNew: () => void;
  menus: Record<string, string>;
  keepOrder?: boolean;
  /** The list goes on past what is loaded: the end of a group, or the button below, loads the next page. */
  more?: BucketMore;
}) {
  const groups = useMemo(() => groupTasks(tasks, keepOrder), [tasks, keepOrder]);
  return (
    <div className="grid gap-8 xl:grid-cols-[minmax(0,1fr)_300px]">
      <div className="min-w-0 space-y-6">
        {loading && <RowSkeleton rows={5} />}
        {!!error && !loading && <EmptyState band={false} title="The pot would not open">{errorMessage(error)}</EmptyState>}
        {!loading && !error && total === 0 && (
          <EmptyState title="Nothing in this pot yet" action={<Button onClick={onNew}>Add the first task</Button>}>
            Put a portion in and the next cook that wakes will scoop it. Tasks can carry a checklist, labels and dependencies.
          </EmptyState>
        )}
        {!loading && total > 0 && tasks.length === 0 && (
          <EmptyState title="No task matches those filters" action={<Button variant="outline" onClick={onClear}>Clear filters</Button>}>
            {filtered ? "Loosen a chip or two and the portions come back." : ""}
          </EmptyState>
        )}
        {tasks.length > 0 &&
          BUCKETS.map((b) =>
            groups[b.id].length > 0 ? (
              <TaskGroup
                key={b.id}
                bucket={b.id}
                tasks={groups[b.id]}
                selected={selected}
                onOpen={onOpen}
                menus={menus}
                defaultOpen={b.id !== "cancelled"}
                onEndReached={more?.has ? more.load : undefined}
              />
            ) : null,
          )}
        {more?.has && (
          <div className="flex justify-center">
            <Button type="button" variant="ghost" size="sm" disabled={more.loading} onClick={more.load} className="text-[12.5px] text-muted-foreground">
              {more.loading && <Loader2 className="animate-spin" />}
              {more.loading ? "Loading" : "Show more"}
            </Button>
          </div>
        )}
      </div>
      <aside aria-label="From the kitchen" className="hidden xl:block">
        <div className="rounded-[14px] border border-border bg-card">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <h2 className="text-[15px]">From the kitchen</h2>
            <Link to={`/pots/${potKey}/activity`} className="text-[12px] text-primary hover:underline">
              all activity
            </Link>
          </div>
          <div className="px-4">
            <EventFeed events={events} compact limit={14} empty="Quiet kitchen. Events show up here as cooks work." />
          </div>
        </div>
      </aside>
    </div>
  );
}

/** The groups of event kinds the activity can be narrowed to: `eventPage` takes a comma list of kinds or `prefix.*`. */
const KIND_GROUPS: { id: string; label: string; kinds: string }[] = [
  { id: "moves", label: "Scoops and moves", kinds: "task.claimed,task.released,task.blocked,task.unblocked,task.lease_expired,task.cancelled,task.restored" },
  { id: "served", label: "Served", kinds: "task.completed" },
  { id: "checklist", label: "Checklists", kinds: "item.checked" },
  { id: "comments", label: "Comments", kinds: "task.commented" },
  { id: "pantry", label: "Pantry", kinds: "memory.*" },
  { id: "menus", label: "Menus", kinds: "menu.*" },
  { id: "questions", label: "Questions", kinds: "question.*" },
  { id: "pass", label: "Pass", kinds: "pass.posted" },
  { id: "files", label: "Files", kinds: "file.*" },
  { id: "restaurant", label: "Restaurant", kinds: "restaurant.decided,wake.failed" },
  { id: "edits", label: "New and edited", kinds: "task.created,task.updated" },
];
const KIND_OPTIONS: FacetOption[] = KIND_GROUPS.map((g) => ({ value: g.id, label: g.label }));
const ACTIVITY_FILTERS = { kind: { type: "string" }, actor: { type: "string" } } as const;

/** A pot's activity, newest first: the server narrows it by kind and actor and pages it as the page scrolls. */
function ActivityView({ project }: { project: ProjectSummary }) {
  const f = useUrlFilters(ACTIVITY_FILTERS);
  const kinds = useStaticSource(KIND_OPTIONS);
  const actor = useActorFacet(project);
  const facets = useMemo<FacetDef[]>(() => [{ id: "kind", label: "Kind", source: kinds, searchable: false }, actor], [kinds, actor]);
  const source = useEventsSource(project.key, { kinds: KIND_GROUPS.find((g) => g.id === f.values.kind)?.kinds ?? null, actor: f.values.actor || null });
  const filtered = !!(f.values.kind || f.values.actor);

  return (
    <div className="mx-auto max-w-3xl">
      <FilterBar hideSearch q="" onQ={() => {}} facets={facets} values={{ kind: f.values.kind, actor: f.values.actor }} onChange={(p) => f.set(p as never)} onClear={f.clear} />
      <div className="mt-3 rounded-[14px] border border-border bg-card px-4 md:mt-4 md:px-5">
        {source.isLoading && source.items.length === 0 ? (
          <div className="py-6">
            <RowSkeleton rows={3} />
          </div>
        ) : source.error && source.items.length === 0 ? (
          <EmptyState band={false} title="The activity would not load" className="border-0">
            {errorMessage(source.error)}
          </EmptyState>
        ) : source.items.length === 0 ? (
          <EmptyState band={false} title="Nothing to report" className="border-0">
            {filtered ? "No event matches those filters." : "Events show up here as cooks scoop, tick and serve."}
          </EmptyState>
        ) : (
          <EventFeed events={source.items} newestFirst hasOlder={!!source.hasNextPage} loadingOlder={source.isFetchingNextPage} onLoadOlder={source.fetchNextPage} />
        )}
      </div>
    </div>
  );
}
