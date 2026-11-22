import { useEffect, useMemo, useState } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";
import type { MenuView, ProjectSummary, TaskView } from "@corntchap/api";
import { Bookmark, Check, ChevronDown, Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useProc, useProjects, useQuestions } from "@/lib/data";
import { useLive } from "@/lib/live";
import { useUi } from "@/lib/ui";
import { BUCKETS, bucketOf, plural, type Bucket } from "@/lib/format";
import { lastView, loadViews, rememberView, saveViews, type BaseView, type Filters, type SavedView } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
import { errorMessage } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EmptyState, RowSkeleton, BUCKET_DOT } from "@/components/bits";
import { PaletteButton } from "@/components/shell";
import { RefreshButton } from "@/components/refresh-button";
import { EventFeed } from "@/components/event-feed";
import { TaskCard, TaskGroup, groupTasks } from "@/components/task-views";
import { TaskSheet } from "@/components/task-sheet";
import { NewMemoryDialog, PantryView } from "@/components/pantry";
import { MenuDialog, MenuStrip, MenusView, useMenus } from "@/components/menus";
import { FilesView } from "@/components/files";
import { AskQuestionDialog, QuestionsView } from "@/components/questions";

const VIEWS: { id: BaseView; label: string }[] = [
  { id: "list", label: "List" },
  { id: "board", label: "Board" },
  { id: "activity", label: "Activity" },
  { id: "pantry", label: "Pantry" },
  { id: "menus", label: "Menus" },
  { id: "questions", label: "Questions" },
  { id: "files", label: "Files" },
];

/** The views that show tasks (and so take filters). */
const TASK_VIEWS: BaseView[] = ["list", "board"];

function readFilters(p: URLSearchParams): Filters {
  const f: Filters = {};
  for (const k of ["status", "label", "holder", "menu", "q"] as const) {
    const v = p.get(k);
    if (v) f[k] = v;
  }
  return f;
}

function applyFilters(tasks: TaskView[], f: Filters): TaskView[] {
  const q = f.q?.trim().toLowerCase();
  return tasks.filter((t) => {
    if (f.status && bucketOf(t) !== f.status) return false;
    if (f.label && !t.labels.includes(f.label)) return false;
    if (f.holder && t.claimed_by !== f.holder) return false;
    if (f.menu && (f.menu === "-" ? !!t.menu : t.menu !== f.menu)) return false;
    if (q && !`${t.id} ${t.title}`.toLowerCase().includes(q)) return false;
    return true;
  });
}

export function PotRedirect() {
  const projects = useProjects();
  const { setPaletteOpen: _p, openNewTask } = useUi();
  if (projects.isLoading) {
    return (
      <div className="px-8 py-10">
        <RowSkeleton />
      </div>
    );
  }
  const first = projects.data?.[0];
  if (first) return <Navigate to={`/pots/${first.key}/${lastView(first.key)}`} replace />;
  return (
    <div className="mx-auto max-w-2xl px-5 py-16">
      <EmptyState
        title="The pot is empty"
        action={<Button onClick={() => openNewTask()}>New task</Button>}
      >
        No pots yet. An admin starts one under Pots, then tasks can go in and cooks can scoop them.
      </EmptyState>
    </div>
  );
}

export function KitchenPage() {
  const { key = "", view = "list" } = useParams();
  const potKey = key.toUpperCase();
  const base: BaseView = view === "board" || view === "activity" || view === "pantry" || view === "menus" || view === "questions" || view === "files" ? view : "list";
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const openTask = useOpenTask();
  const { openNewTask } = useUi();
  const projects = useProjects();
  const project = projects.data?.find((p) => p.key === potKey);
  const tasks = useProc("projectTasks", { project: potKey }, { enabled: !!project });
  const { events } = useLive();
  const filters = readFilters(params);
  const selected = params.get("task");
  const [newMemory, setNewMemory] = useState(false);
  const [asking, setAsking] = useState(false);
  const openQuestions = useQuestions(potKey, "open", !!project).data?.length ?? 0;
  const [menuDialog, setMenuDialog] = useState<{ open: boolean; menu: MenuView | null }>({ open: false, menu: null });
  const menus = useMenus(potKey, !!project);

  useEffect(() => {
    if (project) rememberView(potKey, base);
    document.title = project ? `${project.name} · corntchap` : "corntchap";
  }, [potKey, base, project]);

  // `?new=1` (from the command palette) opens the New menu dialog on the Menus tab
  useEffect(() => {
    if (base === "menus" && params.get("new")) {
      setMenuDialog({ open: true, menu: null });
      setParams((p) => { const n = new URLSearchParams(p); n.delete("new"); return n; }, { replace: true });
    }
  }, [base, params, setParams]);

  const setFilter = (k: keyof Filters, v: string | undefined) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const clearFilters = () =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        for (const k of ["status", "label", "holder", "menu", "q"]) n.delete(k);
        return n;
      },
      { replace: true },
    );
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
      <div className="px-5 py-8 lg:px-8">
        <RowSkeleton />
      </div>
    );
  }
  if (!project) {
    return (
      <div className="mx-auto max-w-2xl px-5 py-16">
        <EmptyState title={`No pot called ${potKey}`} action={<Button asChild variant="outline"><Link to="/">Back to the kitchen</Link></Button>}>
          Either it does not exist or you are not on its team.
        </EmptyState>
      </div>
    );
  }

  const all = tasks.data ?? [];
  const visible = applyFilters(all, filters);
  const potEvents = events.filter((e) => e.project === potKey);
  const selectedMenu = filters.menu ? menus.menus.find((m) => m.key === filters.menu) : undefined;
  // while one menu is selected every row is on it, so the chip would only repeat the strip
  const chipNames = filters.menu ? {} : menus.names;

  return (
    <div className="px-5 pt-6 pb-16 lg:px-8">
      <PotHeader
        project={project}
        base={base}
        filters={filters}
        setFilter={setFilter}
        clearFilters={clearFilters}
        tasks={all}
        menus={menus.menus}
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
        applyView={(v) => {
          const qs = new URLSearchParams();
          for (const [k, val] of Object.entries(v.filters)) if (val) qs.set(k, val);
          void navigate(`/pots/${potKey}/${v.base}${qs.size ? `?${qs}` : ""}`);
        }}
      />

      {selectedMenu && (base === "list" || base === "board") && <MenuStrip menu={selectedMenu} />}

      <div className="mt-6">
        {base === "list" && (
          <ListView
            loading={tasks.isLoading}
            error={tasks.error}
            tasks={visible}
            total={all.length}
            filtered={Object.keys(filters).length > 0}
            selected={selected}
            onOpen={openTask}
            events={potEvents}
            potKey={potKey}
            onClear={clearFilters}
            onNew={() => openNewTask(potKey, filters.menu)}
            menus={chipNames}
          />
        )}
        {base === "board" && <BoardView loading={tasks.isLoading} tasks={visible} selected={selected} onOpen={openTask} onNew={() => openNewTask(potKey, filters.menu)} total={all.length} menus={chipNames} />}
        {base === "activity" && <ActivityView events={potEvents} />}
        {base === "pantry" && <PantryView potKey={potKey} onNew={() => setNewMemory(true)} />}
        {base === "files" && <FilesView potKey={potKey} />}
        {base === "questions" && <QuestionsView potKey={potKey} onAsk={() => setAsking(true)} />}
        {base === "menus" && (
          <MenusView
            potKey={potKey}
            menus={menus.menus}
            open={menus.open}
            served={menus.served}
            loading={menus.isLoading}
            onNew={() => setMenuDialog({ open: true, menu: null })}
            onEdit={(m) => setMenuDialog({ open: true, menu: m })}
          />
        )}
      </div>

      <TaskSheet id={selected} onClose={closeSheet} />
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

function PotHeader({
  project,
  base,
  filters,
  setFilter,
  clearFilters,
  tasks,
  menus,
  onNew,
  applyView,
  openQuestions,
}: {
  project: ProjectSummary;
  base: BaseView;
  filters: Filters;
  setFilter: (k: keyof Filters, v: string | undefined) => void;
  clearFilters: () => void;
  tasks: TaskView[];
  menus: MenuView[];
  onNew: () => void;
  applyView: (v: SavedView) => void;
  openQuestions: number;
}) {
  const loc = useLocation();
  const [saved, setSaved] = useState<SavedView[]>(() => loadViews(project.key));
  useEffect(() => setSaved(loadViews(project.key)), [project.key]);

  const labels = useMemo(() => [...new Set(tasks.flatMap((t) => t.labels))].sort(), [tasks]);
  const holders = useMemo(() => [...new Set(tasks.map((t) => t.claimed_by).filter((h): h is string => !!h))].sort(), [tasks]);
  const active = Object.values(filters).some(Boolean);

  return (
    <header>
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0">
          <p className="label-caps flex items-center gap-2">
            <span className="rounded bg-corn px-1.5 py-px font-bold !text-corn-foreground">{project.key}</span>
            {project.team && <span>team {project.team}</span>}
          </p>
          <h1 className="mt-1.5 truncate text-[30px] leading-[1.1] sm:text-[34px]">{project.name}</h1>
          <p className="mt-1.5 text-[13px] text-muted-foreground">
            <b className="font-semibold text-foreground">{project.claimed}</b> on the fire ·{" "}
            <b className="font-semibold text-foreground">{project.ready}</b> ready to scoop ·{" "}
            <b className="font-semibold text-foreground">{project.blocked}</b> waiting ·{" "}
            <b className="font-semibold text-foreground">{project.done}</b> served
            {project.tossed > 0 && (
              <>
                {" "}· <b className="font-semibold text-foreground">{project.tossed}</b> tossed
              </>
            )}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <PaletteButton />
          <RefreshButton />
          <Button onClick={onNew}>
            <Plus /> {base === "pantry" ? "Stock the pantry" : base === "menus" ? "New menu" : base === "questions" ? "Ask a question" : "New task"}
          </Button>
        </div>
      </div>

      <div className="mt-5 flex flex-wrap items-center gap-x-4 gap-y-3">
        <nav aria-label="Views" className="flex max-w-full items-center gap-1 overflow-x-auto rounded-xl bg-muted p-1">
          {VIEWS.map((v) => (
            <Link
              key={v.id}
              to={{ pathname: `/pots/${project.key}/${v.id}`, search: !TASK_VIEWS.includes(v.id) || !TASK_VIEWS.includes(base) ? "" : loc.search }}
              aria-current={base === v.id && !(active && TASK_VIEWS.includes(base)) ? "page" : undefined}
              className={cn(
                "rounded-lg px-3.5 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground",
                base === v.id && "bg-card text-foreground shadow-sm",
              )}
            >
              {v.label}
              {v.id === "questions" && openQuestions > 0 && (
                <span className="relative ml-1.5 inline-flex min-w-4 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10.5px] leading-4 font-bold text-primary-foreground">
                  <span aria-hidden="true">{openQuestions}</span>
                  <span className="sr-only"> open {openQuestions === 1 ? "question" : "questions"}</span>
                </span>
              )}
            </Link>
          ))}
          {saved.map((v) => (
            <button
              key={v.id}
              type="button"
              onClick={() => applyView(v)}
              className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground hover:text-foreground"
            >
              <Bookmark className="size-3" aria-hidden="true" /> {v.name}
            </button>
          ))}
          <AddView
            pot={project.key}
            base={base}
            filters={filters}
            onSaved={(v) => setSaved(v)}
            saved={saved}
          />
        </nav>

        {TASK_VIEWS.includes(base) && (
          <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Filters">
            <FilterChip
              label="Menu"
              value={filters.menu === "-" ? "No menu" : filters.menu ? (menus.find((m) => m.key === filters.menu)?.name ?? filters.menu) : undefined}
              onClear={() => setFilter("menu", undefined)}
            >
              <DropdownMenuRadioGroup value={filters.menu ?? ""} onValueChange={(v) => setFilter("menu", v || undefined)}>
                <DropdownMenuRadioItem value="">All menus</DropdownMenuRadioItem>
                {menus
                  .filter((m) => m.status !== "served")
                  .map((m) => (
                    <DropdownMenuRadioItem key={m.key} value={m.key}>
                      {m.name}
                      <span className="ml-auto pl-3 font-mono text-[11px] text-muted-foreground">
                        {m.done}/{m.total}
                      </span>
                    </DropdownMenuRadioItem>
                  ))}
                <DropdownMenuRadioItem value="-">No menu</DropdownMenuRadioItem>
                {menus.some((m) => m.status === "served") && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuLabel className="label-caps py-1">Served</DropdownMenuLabel>
                    {menus
                      .filter((m) => m.status === "served")
                      .map((m) => (
                        <DropdownMenuRadioItem key={m.key} value={m.key}>
                          {m.name}
                        </DropdownMenuRadioItem>
                      ))}
                  </>
                )}
              </DropdownMenuRadioGroup>
              <DropdownMenuSeparator />
              <DropdownMenuItem asChild>
                <Link to={`/pots/${project.key}/menus`}>Manage menus…</Link>
              </DropdownMenuItem>
            </FilterChip>
            <FilterChip
              label="Status"
              value={filters.status ? BUCKETS.find((b) => b.id === filters.status)?.label : undefined}
              onClear={() => setFilter("status", undefined)}
            >
              <DropdownMenuRadioGroup value={filters.status ?? ""} onValueChange={(v) => setFilter("status", v || undefined)}>
                {BUCKETS.map((b) => (
                  <DropdownMenuRadioItem key={b.id} value={b.id}>
                    <span className={cn("size-2 rounded-full", BUCKET_DOT[b.id])} /> {b.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </FilterChip>
            <FilterChip label="Label" value={filters.label} onClear={() => setFilter("label", undefined)} disabled={labels.length === 0}>
              <DropdownMenuRadioGroup value={filters.label ?? ""} onValueChange={(v) => setFilter("label", v || undefined)}>
                {labels.map((l) => (
                  <DropdownMenuRadioItem key={l} value={l}>
                    {l}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </FilterChip>
            <FilterChip label="Cook" value={filters.holder} onClear={() => setFilter("holder", undefined)} disabled={holders.length === 0}>
              <DropdownMenuRadioGroup value={filters.holder ?? ""} onValueChange={(v) => setFilter("holder", v || undefined)}>
                {holders.map((h) => (
                  <DropdownMenuRadioItem key={h} value={h}>
                    {h}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </FilterChip>
            <div className="relative">
              <Input
                aria-label="Filter tasks by text"
                value={filters.q ?? ""}
                onChange={(e) => setFilter("q", e.target.value || undefined)}
                placeholder="Filter…"
                className="h-8 w-32 rounded-full pr-2 pl-3 text-[13px]"
              />
            </div>
            {active && (
              <button type="button" onClick={clearFilters} className="text-[12.5px] text-primary hover:underline">
                Clear filters
              </button>
            )}
          </div>
        )}
      </div>
    </header>
  );
}

function FilterChip({
  label,
  value,
  onClear,
  children,
  disabled,
}: {
  label: string;
  value: string | undefined;
  onClear: () => void;
  children: React.ReactNode;
  disabled?: boolean;
}) {
  return (
    <DropdownMenu>
      <div
        className={cn(
          "inline-flex h-8 items-center rounded-full border text-[12.5px] transition-colors",
          value ? "border-primary/60 bg-primary/10" : "border-input bg-card hover:border-foreground/30",
          disabled && "opacity-50",
        )}
      >
        <DropdownMenuTrigger disabled={disabled} className="inline-flex h-full items-center gap-1.5 rounded-full pr-2 pl-3">
          <span className="text-muted-foreground">{label}</span>
          {value && <span className="font-medium">{value}</span>}
          <ChevronDown className="size-3 text-muted-foreground" aria-hidden="true" />
        </DropdownMenuTrigger>
        {value && (
          <button type="button" onClick={onClear} aria-label={`Clear ${label} filter`} className="mr-1.5 rounded-full p-0.5 text-muted-foreground hover:text-foreground">
            <X className="size-3" />
          </button>
        )}
      </div>
      <DropdownMenuContent align="start" className="min-w-44">
        <DropdownMenuLabel className="label-caps py-1">{label}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AddView({
  pot,
  base,
  filters,
  saved,
  onSaved,
}: {
  pot: string;
  base: BaseView;
  filters: Filters;
  saved: SavedView[];
  onSaved: (v: SavedView[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="rounded-lg px-3 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground hover:text-foreground"
        >
          + View
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 space-y-3">
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (!name.trim()) return;
            const next = [...saved, { id: crypto.randomUUID(), name: name.trim(), base, filters }];
            saveViews(pot, next);
            onSaved(next);
            setName("");
            setOpen(false);
          }}
        >
          <div>
            <p className="font-display text-[15px]">Save this view</p>
            <p className="mt-0.5 text-[12px] text-muted-foreground">
              Keeps the {base} layout and the current filters for {pot}, in this browser.
            </p>
          </div>
          <Input aria-label="View name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Frontend on the fire" autoFocus />
          <div className="flex justify-between gap-2">
            {saved.length > 0 ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  saveViews(pot, []);
                  onSaved([]);
                  setOpen(false);
                }}
              >
                Remove saved
              </Button>
            ) : (
              <span />
            )}
            <Button type="submit" size="sm" disabled={!name.trim()}>
              <Check /> Save view
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
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
}) {
  const groups = useMemo(() => groupTasks(tasks), [tasks]);
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
              />
            ) : null,
          )}
      </div>
      <aside aria-label="From the kitchen" className="hidden xl:block">
        <div className="sticky top-6 rounded-[14px] border border-border bg-card">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <h2 className="text-[15px]">From the kitchen</h2>
            <Link to={`/pots/${potKey}/activity`} className="text-[12px] text-primary hover:underline">
              all activity
            </Link>
          </div>
          <div className="max-h-[calc(100dvh-9rem)] overflow-y-auto px-4 scrollbar-thin">
            <EventFeed events={events} compact limit={14} empty="Quiet kitchen. Events show up here as cooks work." />
          </div>
        </div>
      </aside>
    </div>
  );
}

const BOARD_COLS: Bucket[] = ["ready", "claimed", "waiting", "done", "cancelled"];

function BoardView({
  loading,
  tasks,
  selected,
  onOpen,
  onNew,
  total,
  menus,
}: {
  loading: boolean;
  tasks: TaskView[];
  selected: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  total: number;
  menus: Record<string, string>;
}) {
  const groups = useMemo(() => groupTasks(tasks), [tasks]);
  if (loading) return <RowSkeleton rows={3} />;
  if (total === 0) {
    return (
      <EmptyState title="The board is bare" action={<Button onClick={onNew}>New task</Button>}>
        Five columns: ready to scoop, on the fire, waiting on a human or a dependency, served, and tossed.
      </EmptyState>
    );
  }
  return (
    <div className="-mx-5 overflow-x-auto px-5 pb-3 lg:-mx-8 lg:px-8 scrollbar-thin">
      <div className="grid min-w-[1000px] grid-cols-[repeat(4,minmax(180px,1fr))_minmax(150px,0.75fr)] gap-3.5 xl:min-w-0">
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          return (
            <section
              key={id}
              aria-labelledby={`col-${id}`}
              className={cn("flex max-h-[calc(100dvh-14rem)] min-h-40 flex-col rounded-[14px] bg-column p-2.5", id === "cancelled" && "opacity-80")}
            >
              <h2 id={`col-${id}`} className="flex items-center gap-2 px-1.5 pt-1 pb-2.5 text-[15px]">
                <span className={cn("size-2.5 rounded-full", BUCKET_DOT[id], id === "claimed" && "live-dot")} aria-hidden="true" />
                {meta.label}
                <span className="font-mono text-[11.5px] font-normal text-muted-foreground" style={{ fontFamily: "var(--font-mono)" }}>
                  {col.length}
                </span>
              </h2>
              <div className="scrollbar-thin flex-1 space-y-2 overflow-y-auto pr-0.5">
                {col.length === 0 && <p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">Nothing here.</p>}
                {col.map((t) => (
                  <TaskCard key={t.id} task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />
                ))}
              </div>
            </section>
          );
        })}
      </div>
      <p className="mt-3 text-[12px] text-muted-foreground">
        Cooks move cards by scooping and serving; the board is read-only so a lease is never broken by a stray drag.
      </p>
    </div>
  );
}

const KIND_GROUPS: { id: string; label: string; match: (k: string) => boolean }[] = [
  { id: "all", label: "Everything", match: () => true },
  { id: "moves", label: "Scoops and moves", match: (k) => ["task.claimed", "task.released", "task.blocked", "task.unblocked", "task.lease_expired", "task.cancelled", "task.restored"].includes(k) },
  { id: "served", label: "Served", match: (k) => k === "task.completed" },
  { id: "checklist", label: "Checklists", match: (k) => k === "item.checked" },
  { id: "comments", label: "Comments", match: (k) => k === "task.commented" },
  { id: "pantry", label: "Pantry", match: (k) => k.startsWith("memory.") },
  { id: "menus", label: "Menus", match: (k) => k.startsWith("menu.") },
  { id: "questions", label: "Questions", match: (k) => k.startsWith("question.") },
  { id: "files", label: "Files", match: (k) => k.startsWith("file.") },
  { id: "edits", label: "New and edited", match: (k) => k === "task.created" || k === "task.updated" },
];

function ActivityView({ events }: { events: ReturnType<typeof useLive>["events"] }) {
  const [kind, setKind] = useState("all");
  const [actor, setActor] = useState<string>("");
  const { ready } = useLive();
  const actors = useMemo(() => [...new Set(events.map((e) => e.actor).filter((a): a is string => !!a))].sort(), [events]);
  const filtered = useMemo(() => {
    const m = KIND_GROUPS.find((g) => g.id === kind)!.match;
    return events.filter((e) => m(e.kind) && (!actor || e.actor === actor));
  }, [events, kind, actor]);

  return (
    <div className="mx-auto max-w-3xl">
      <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Activity filters">
        {KIND_GROUPS.map((g) => (
          <button
            key={g.id}
            type="button"
            aria-pressed={kind === g.id}
            onClick={() => setKind(g.id)}
            className={cn(
              "h-8 rounded-full border px-3 text-[12.5px] transition-colors",
              kind === g.id ? "border-primary/60 bg-primary/10 font-medium" : "border-input bg-card text-muted-foreground hover:border-foreground/30",
            )}
          >
            {g.label}
          </button>
        ))}
        <DropdownMenu>
          <DropdownMenuTrigger className={cn("inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-[12.5px]", actor ? "border-primary/60 bg-primary/10" : "border-input bg-card text-muted-foreground")}>
            {actor || "Anyone"} <ChevronDown className="size-3" aria-hidden="true" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuCheckboxItem checked={!actor} onCheckedChange={() => setActor("")}>
              Anyone
            </DropdownMenuCheckboxItem>
            {actors.map((a) => (
              <DropdownMenuCheckboxItem key={a} checked={actor === a} onCheckedChange={() => setActor(a)}>
                {a}
              </DropdownMenuCheckboxItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <div className="mt-4 rounded-[14px] border border-border bg-card px-5">
        {!ready ? (
          <div className="py-6"><RowSkeleton rows={3} /></div>
        ) : filtered.length === 0 ? (
          <EmptyState band={false} title="Nothing to report" className="border-0">
            {events.length === 0 ? "Events show up here as cooks scoop, tick and serve." : plural(events.length, "event") + " hidden by the filters."}
          </EmptyState>
        ) : (
          <EventFeed events={filtered} limit={200} />
        )}
      </div>
    </div>
  );
}
