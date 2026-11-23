import { useEffect, useMemo, useRef, useState } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";
import type { MenuView, ProjectSummary, TaskView } from "@corntchap/api";
import type { ViewOut } from "@corntchap/api";
import { Check, ChevronDown, Pencil, Plus, Settings2, SlidersHorizontal, Trash2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useProc, useProjects, useQuestions } from "@/lib/data";
import { useLive } from "@/lib/live";
import { useUi } from "@/lib/ui";
import { BUCKETS, bucketOf, plural, type Bucket } from "@/lib/format";
import { forgetView, isBaseView, lastPot, lastView, rememberPot, rememberView, type BaseView, type Filters } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
import { useIsPhone } from "@/lib/media";
import { errorMessage } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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
import { PassView } from "@/components/pass";
import { AskQuestionDialog, QuestionsView } from "@/components/questions";
import { DeleteViewDialog, ViewDialog, layoutOf, useViews } from "@/components/views";
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

function readFilters(p: URLSearchParams): Filters {
  const f: Filters = {};
  for (const k of ["status", "label", "repo", "holder", "menu", "q"] as const) {
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
    if (f.repo && !t.repos.includes(f.repo)) return false;
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
  const remembered = lastPot();
  const first = projects.data?.find((p) => p.key === remembered) ?? projects.data?.[0];
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
  const { key = "", view, viewKey } = useParams();
  const potKey = key.toUpperCase();
  const base: BaseView = isBaseView(view) ? view : "board";
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const openTask = useOpenTask();
  const { openNewTask } = useUi();
  const projects = useProjects();
  const project = projects.data?.find((p) => p.key === potKey);
  const tasks = useProc("projectTasks", { project: potKey }, { enabled: !!project && !viewKey });
  const saved = useViews(potKey, !!project);
  const savedView = viewKey ? saved.views.find((v) => v.key === viewKey) : undefined;
  const [viewDialog, setViewDialog] = useState<{ open: boolean; view: ViewOut | null }>({ open: false, view: null });
  const [deleting, setDeleting] = useState<ViewOut | null>(null);
  const taskView = !!viewKey || base === "board";
  const { events } = useLive();
  const filters = readFilters(params);
  const selected = params.get("task");
  const [newMemory, setNewMemory] = useState(false);
  const [asking, setAsking] = useState(false);
  const openQuestions = useQuestions(potKey, "open", !!project).data?.length ?? 0;
  const [menuDialog, setMenuDialog] = useState<{ open: boolean; menu: MenuView | null }>({ open: false, menu: null });
  const menus = useMenus(potKey, !!project);

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
        for (const k of ["status", "label", "repo", "holder", "menu", "q"]) n.delete(k);
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
      <div className="px-4 py-6 md:px-5 md:py-8 lg:px-8">
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
    <div className="px-4 pt-3 pb-6 md:px-5 md:pt-6 md:pb-16 lg:px-8">
      <PotHeader
        project={project}
        base={base}
        viewKey={viewKey}
        saved={saved.views}
        taskView={taskView}
        onNewView={() => setViewDialog({ open: true, view: null })}
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
        {!viewKey && base === "board" && <BoardView loading={tasks.isLoading} tasks={visible} selected={selected} onOpen={openTask} onNew={() => openNewTask(potKey, filters.menu)} total={all.length} menus={chipNames} />}
        {!viewKey && base === "activity" && <ActivityView events={potEvents} />}
        {!viewKey && base === "pantry" && <PantryView potKey={potKey} onNew={() => setNewMemory(true)} />}
        {!viewKey && base === "files" && <FilesView potKey={potKey} />}
        {!viewKey && base === "pass" && <PassView potKey={potKey} />}
        {!viewKey && base === "questions" && <QuestionsView potKey={potKey} onAsk={() => setAsking(true)} />}
        {!viewKey && base === "menus" && (
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
    <div className={cn("flex items-center", withLabels ? "flex-wrap gap-1.5" : "gap-1")} role="group" aria-label="Filter by status">
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
  saved: ViewOut[];
  taskView: boolean;
  onNewView: () => void;
  filters: Filters;
  setFilter: (k: keyof Filters, v: string | undefined) => void;
  clearFilters: () => void;
  tasks: TaskView[];
  menus: MenuView[];
  onNew: () => void;
  openQuestions: number;
}

function PotHeader(props: HeaderProps) {
  return useIsPhone() ? <PhoneHeader {...props} /> : <WideHeader {...props} />;
}

function PhoneHeader({ project, base, viewKey, saved, taskView, onNewView, filters, setFilter, clearFilters, tasks, menus, onNew }: HeaderProps) {
  const [sheet, setSheet] = useState(false);
  const activeCount = Object.values(filters).filter(Boolean).length;
  const current = viewKey ? (saved.find((v) => v.key === viewKey)?.name ?? "View") : "Board";

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
    <header className="flex items-center gap-2">
      <DropdownMenu>
        <DropdownMenuTrigger className="inline-flex h-10 max-w-32 shrink-0 items-center gap-1 rounded-full border border-border bg-card pr-2.5 pl-3.5 text-[13px] font-medium active:bg-muted">
          <span className="truncate">{current}</span>
          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-48">
          <DropdownMenuLabel className="label-caps py-1">Views</DropdownMenuLabel>
          <DropdownMenuItem asChild className="min-h-11">
            <Link to={`/pots/${project.key}/board`}>
              Board {!viewKey && <Check className="ml-auto size-4" aria-hidden="true" />}
            </Link>
          </DropdownMenuItem>
          {saved.map((v) => (
            <DropdownMenuItem key={v.key} asChild className="min-h-11">
              <Link to={`/pots/${project.key}/v/${v.key}`}>
                <span className="truncate">{v.name}</span>
                {viewKey === v.key && <Check className="ml-auto size-4" aria-hidden="true" />}
              </Link>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onNewView} className="min-h-11">
            <Plus /> New view
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {/* the board has its own status switcher under this line; lists filter by status here */}
      <div className="scrollbar-none min-w-0 flex-1 overflow-x-auto">
        {viewKey && <StatusChips project={project} status={filters.status} onPick={(b) => setFilter("status", b)} withLabels={false} />}
      </div>
      <button
        type="button"
        onClick={() => setSheet(true)}
        aria-label={activeCount ? `Display and filter, ${activeCount} active` : "Display and filter"}
        className={cn(
          "relative inline-flex size-10 shrink-0 items-center justify-center rounded-full border active:bg-muted",
          activeCount ? "border-primary/60 bg-primary/10" : "border-border bg-card",
        )}
      >
        <SlidersHorizontal className="size-4" aria-hidden="true" />
        {activeCount > 0 && (
          <span className="absolute -top-1 -right-1 grid min-w-4 place-items-center rounded-full bg-primary px-1 font-mono text-[10px] leading-4 font-bold text-primary-foreground">{activeCount}</span>
        )}
      </button>
      <FilterSheet open={sheet} onOpenChange={setSheet} project={project} filters={filters} setFilter={setFilter} clearFilters={clearFilters} tasks={tasks} menus={menus} />
    </header>
  );
}

function Pill({ on, onClick, children }: { on: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        "inline-flex min-h-10 items-center gap-1.5 rounded-full border px-3.5 text-[14px] transition-colors active:bg-muted",
        on ? "border-primary/60 bg-primary/10 font-medium" : "border-border bg-background",
      )}
    >
      {children}
    </button>
  );
}

/** Phone: Menu, Status, Label, Cook and the text filter in one bottom sheet (Linear's display options). */
function FilterSheet({
  open,
  onOpenChange,
  project,
  filters,
  setFilter,
  clearFilters,
  tasks,
  menus,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  project: ProjectSummary;
  filters: Filters;
  setFilter: (k: keyof Filters, v: string | undefined) => void;
  clearFilters: () => void;
  tasks: TaskView[];
  menus: MenuView[];
}) {
  const labels = useMemo(() => [...new Set(tasks.flatMap((t) => t.labels))].sort(), [tasks]);
  const repos = useMemo(() => [...new Set([...project.repos.map((r) => r.name), ...tasks.flatMap((t) => t.repos)])], [tasks, project.repos]);
  const holders = useMemo(() => [...new Set(tasks.map((t) => t.claimed_by).filter((h): h is string => !!h))].sort(), [tasks]);
  const toggle = (k: keyof Filters, v: string) => setFilter(k, filters[k] === v ? undefined : v);
  const active = Object.values(filters).some(Boolean);
  const section = (title: string, body: React.ReactNode) => (
    <section className="space-y-2">
      <h3 className="label-caps">{title}</h3>
      <div className="flex flex-wrap gap-2">{body}</div>
    </section>
  );
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-sm:flex max-sm:flex-col max-sm:gap-0 max-sm:p-0" onOpenAutoFocus={(e) => e.preventDefault()}>
        <DialogHeader className="flex-row items-center justify-between gap-2 px-5 pt-1 pb-3 text-left max-sm:pt-7">
          <div>
            <DialogTitle>Display and filter</DialogTitle>
            <DialogDescription className="sr-only">Narrow the {project.key} board by menu, status, label, cook or text.</DialogDescription>
          </div>
          <div className="mr-8 flex items-center gap-2">
            <RefreshButton />
            <PotOptions project={project} />
          </div>
        </DialogHeader>
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto border-t border-border px-5 py-4">
          <div className="space-y-2">
            <Label htmlFor="f-q" className="label-caps">Text</Label>
            <Input id="f-q" value={filters.q ?? ""} onChange={(e) => setFilter("q", e.target.value || undefined)} placeholder="Search id or title" className="h-11" />
          </div>
          {section(
            "Status",
            BUCKETS.map((b) => (
              <Pill key={b.id} on={filters.status === b.id} onClick={() => toggle("status", b.id)}>
                <span className={cn("size-2 rounded-full", BUCKET_DOT[b.id])} aria-hidden="true" /> {b.label}
              </Pill>
            )),
          )}
          {section(
            "Menu",
            <>
              {menus
                .filter((m) => m.status !== "served")
                .map((m) => (
                  <Pill key={m.key} on={filters.menu === m.key} onClick={() => toggle("menu", m.key)}>
                    {m.name} <span className="font-mono text-[11px] text-muted-foreground">{m.done}/{m.total}</span>
                  </Pill>
                ))}
              <Pill on={filters.menu === "-"} onClick={() => toggle("menu", "-")}>No menu</Pill>
            </>,
          )}
          {labels.length > 0 && section("Label", labels.map((l) => <Pill key={l} on={filters.label === l} onClick={() => toggle("label", l)}>{l}</Pill>))}
          {repos.length > 0 && section("Repository", repos.map((r) => <Pill key={r} on={filters.repo === r} onClick={() => toggle("repo", r)}>{r}</Pill>))}
          {holders.length > 0 && section("Cook", holders.map((h) => <Pill key={h} on={filters.holder === h} onClick={() => toggle("holder", h)}>{h}</Pill>))}
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

function WideHeader({ project, base, viewKey, saved, taskView, onNewView, filters, setFilter, clearFilters, tasks, menus, onNew, openQuestions }: HeaderProps) {
  const loc = useLocation();
  const labels = useMemo(() => [...new Set(tasks.flatMap((t) => t.labels))].sort(), [tasks]);
  const repos = useMemo(() => [...new Set([...project.repos.map((r) => r.name), ...tasks.flatMap((t) => t.repos)])], [tasks, project.repos]);
  const holders = useMemo(() => [...new Set(tasks.map((t) => t.claimed_by).filter((h): h is string => !!h))].sort(), [tasks]);
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

      <nav aria-label="Views" className="scrollbar-none mt-5 flex max-w-full items-center gap-1 overflow-x-auto rounded-xl bg-muted p-1">
        {HEAD_VIEWS.map(tab)}
        {saved.map((v) => (
          <Link
            key={v.key}
            to={{ pathname: `/pots/${project.key}/v/${v.key}`, search: taskView ? loc.search : "" }}
            aria-current={viewKey === v.key && !active ? "page" : undefined}
            className={cn(
              "shrink-0 rounded-lg px-3.5 py-1.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground pointer-coarse:py-2.5",
              viewKey === v.key && "bg-card text-foreground shadow-sm",
            )}
          >
            {v.name}
          </Link>
        ))}
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
        <div className="scrollbar-none mt-3 flex items-center gap-2 overflow-x-auto pb-0.5" role="group" aria-label="Filters">
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
          {repos.length > 0 && (
            <FilterChip label="Repository" value={filters.repo} onClear={() => setFilter("repo", undefined)}>
              <DropdownMenuRadioGroup value={filters.repo ?? ""} onValueChange={(v) => setFilter("repo", v || undefined)}>
                {repos.map((r) => (
                  <DropdownMenuRadioItem key={r} value={r}>
                    {r}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </FilterChip>
          )}
          <FilterChip label="Cook" value={filters.holder} onClear={() => setFilter("holder", undefined)} disabled={holders.length === 0}>
            <DropdownMenuRadioGroup value={filters.holder ?? ""} onValueChange={(v) => setFilter("holder", v || undefined)}>
              {holders.map((h) => (
                <DropdownMenuRadioItem key={h} value={h}>
                  {h}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </FilterChip>
          <Input
            aria-label="Filter tasks by text"
            value={filters.q ?? ""}
            onChange={(e) => setFilter("q", e.target.value || undefined)}
            placeholder="Filter…"
            className="h-8 w-36 shrink-0 rounded-full pr-2 pl-3 text-[13px]"
          />
          {active && (
            <button type="button" onClick={clearFilters} className="shrink-0 px-1 text-[12.5px] whitespace-nowrap text-primary hover:underline">
              Clear filters
            </button>
          )}
        </div>
      )}
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
  const run = useProc("viewRun", { project: potKey, key: view?.key ?? null }, { enabled: !!view });
  if (loadingViews) return <RowSkeleton rows={4} />;
  if (!view) {
    return (
      <EmptyState title="No such view" action={<Button asChild variant="outline"><Link to={`/pots/${potKey}/board`}>Back to the board</Link></Button>}>
        It may have been deleted, or it was never made on this pot.
      </EmptyState>
    );
  }
  const tasks = run.data ?? [];
  const visible = applyFilters(tasks, filters);
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
    />
  ) : (
    <BoardView loading={run.isLoading} tasks={visible} selected={selected} onOpen={onOpen} onNew={() => undefined} total={tasks.length} menus={menus} keepOrder />
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
  keepOrder = false,
}: {
  loading: boolean;
  tasks: TaskView[];
  selected: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  total: number;
  menus: Record<string, string>;
  keepOrder?: boolean;
}) {
  const groups = useMemo(() => groupTasks(tasks, keepOrder), [tasks, keepOrder]);
  const phone = useIsPhone();
  if (loading) return <BoardSkeleton phone={phone} />;
  if (total === 0) {
    return (
      <EmptyState title="The board is bare" action={<Button onClick={onNew}>New task</Button>}>
        Five columns: ready to scoop, on the fire, waiting on a human or a dependency, served, and tossed.
      </EmptyState>
    );
  }
  if (phone) return <PhoneBoard groups={groups} selected={selected} onOpen={onOpen} menus={menus} />;
  return (
    <div
      role="region"
      aria-label="Board"
      tabIndex={0}
      className="scrollbar-thin -mx-5 max-h-[calc(100dvh-16rem)] min-h-64 overflow-auto overscroll-contain px-5 pb-3 focus-visible:outline-none lg:-mx-8 lg:px-8"
    >
      <div className="flex w-max min-w-full items-stretch gap-3.5">
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          return (
            <section
              key={id}
              aria-labelledby={`col-${id}`}
              className={cn("flex w-[17.5rem] shrink-0 flex-col rounded-[14px] bg-column p-2.5 sm:w-[19rem]", id === "cancelled" && "opacity-80")}
            >
              <h2 id={`col-${id}`} className="sticky top-0 z-10 -mx-2.5 -mt-2.5 mb-1 flex items-center gap-2 rounded-t-[14px] bg-column px-4 pt-3.5 pb-2.5 text-[15px]">
                <span className={cn("size-2.5 rounded-full", BUCKET_DOT[id], id === "claimed" && "live-dot")} aria-hidden="true" />
                {meta.label}
                <span className="font-mono text-[11.5px] font-normal text-muted-foreground" style={{ fontFamily: "var(--font-mono)" }}>
                  {col.length}
                </span>
              </h2>
              <div className="space-y-2">
                {col.length === 0 && <p className="px-2 py-5 text-center text-[12.5px] text-muted-foreground">Nothing here.</p>}
                {col.map((t) => (
                  <TaskCard key={t.id} task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />
                ))}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}


const BOARD_SHORT: Record<Bucket, string> = { ready: "Ready", claimed: "Fire", waiting: "Waiting", done: "Served", cancelled: "Tossed" };
const BOARD_EMPTY: Record<Bucket, string> = {
  ready: "Nothing is waiting to be scooped.",
  claimed: "No cook has anything on the fire.",
  waiting: "Nothing is stuck.",
  done: "Nothing served yet.",
  cancelled: "Nothing tossed.",
};

function BoardSkeleton({ phone }: { phone: boolean }) {
  return (
    <div className="flex gap-3 overflow-hidden" aria-busy="true" aria-label="Loading the board">
      {Array.from({ length: phone ? 1 : 3 }).map((_, c) => (
        <div key={c} className={cn("shrink-0 space-y-2 rounded-[14px] bg-column p-2.5", phone ? "w-full" : "w-[19rem]")}>
          {Array.from({ length: 3 }).map((__, i) => (
            <div key={i} className="space-y-2.5 rounded-[10px] border border-border bg-card p-3">
              <div className="h-2.5 w-12 animate-pulse rounded bg-muted" />
              <div className="h-3.5 animate-pulse rounded bg-muted" style={{ width: `${88 - i * 14}%` }} />
              <div className="h-2 w-2/3 animate-pulse rounded-full bg-muted" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/** Phone board: one column per screen (snap scrolling) and a sticky segmented control that jumps between them. */
function PhoneBoard({
  groups,
  selected,
  onOpen,
  menus,
}: {
  groups: Record<Bucket, TaskView[]>;
  selected: string | null;
  onOpen: (id: string) => void;
  menus: Record<string, string>;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const start = () => {
    const el = scroller.current;
    return el ? parseFloat(getComputedStyle(el).paddingLeft) || 0 : 0;
  };
  const onScroll = () => {
    const el = scroller.current;
    const first = el?.children[0] as HTMLElement | undefined;
    const second = el?.children[1] as HTMLElement | undefined;
    if (!el || !first || !second) return;
    const step = second.offsetLeft - first.offsetLeft;
    setActive(Math.max(0, Math.min(BOARD_COLS.length - 1, Math.round(el.scrollLeft / step))));
  };
  const jump = (i: number) => {
    const el = scroller.current;
    const col = el?.children[i] as HTMLElement | undefined;
    if (!el || !col) return;
    setActive(i);
    el.scrollTo({ left: col.offsetLeft - start(), behavior: "smooth" });
  };
  return (
    <div>
      <div
        role="tablist"
        aria-label="Board columns"
        className="sticky top-[calc(3.5rem+env(safe-area-inset-top))] z-20 -mx-4 bg-background/95 px-4 pb-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/80"
      >
        <div className="grid grid-cols-5 gap-0.5 rounded-xl bg-muted p-1">
          {BOARD_COLS.map((id, i) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={active === i}
              onClick={() => jump(i)}
              className={cn(
                "flex min-h-10 flex-col items-center justify-center rounded-lg px-0.5 text-[11.5px] leading-tight font-medium transition-colors",
                active === i ? "bg-card text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              <span className="flex items-center gap-1">
                <span className={cn("size-1.5 rounded-full", BUCKET_DOT[id], id === "claimed" && groups[id].length > 0 && "live-dot")} aria-hidden="true" />
                {BOARD_SHORT[id]}
              </span>
              <span className="font-mono text-[11px] tabular-nums" style={{ fontFamily: "var(--font-mono)" }}>{groups[id].length}</span>
            </button>
          ))}
        </div>
      </div>
      <div
        ref={scroller}
        role="region"
        aria-label="Board"
        onScroll={onScroll}
        className="scrollbar-none relative -mx-4 flex snap-x snap-mandatory items-start gap-3 overflow-x-auto overscroll-x-contain scroll-px-4 px-4 pb-2"
      >
        {BOARD_COLS.map((id) => {
          const meta = BUCKETS.find((b) => b.id === id)!;
          const col = groups[id];
          return (
            <section key={id} aria-labelledby={`col-${id}`} className={cn("w-[85vw] shrink-0 snap-start rounded-[14px] bg-column p-2", id === "cancelled" && "opacity-80")}>
              <h2 id={`col-${id}`} className="sr-only">
                {meta.label}, {col.length}
              </h2>
              <div className="space-y-2">
                {col.length === 0 && <p className="px-2 py-8 text-center text-[13px] text-muted-foreground">{BOARD_EMPTY[id]}</p>}
                {col.map((t) => (
                  <TaskCard key={t.id} task={t} selected={selected === t.id} onOpen={onOpen} menus={menus} />
                ))}
              </div>
            </section>
          );
        })}
      </div>
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
  { id: "pass", label: "Pass", match: (k) => k === "pass.posted" },
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
      <div className="scrollbar-none -mx-4 flex items-center gap-2 overflow-x-auto px-4 pb-1 md:mx-0 md:flex-wrap md:overflow-visible md:px-0 md:pb-0" role="group" aria-label="Activity filters">
        {KIND_GROUPS.map((g) => (
          <button
            key={g.id}
            type="button"
            aria-pressed={kind === g.id}
            onClick={() => setKind(g.id)}
            className={cn(
              "h-10 shrink-0 rounded-full border px-3.5 text-[13px] whitespace-nowrap transition-colors md:h-8 md:px-3 md:text-[12.5px]",
              kind === g.id ? "border-primary/60 bg-primary/10 font-medium" : "border-input bg-card text-muted-foreground hover:border-foreground/30",
            )}
          >
            {g.label}
          </button>
        ))}
        <DropdownMenu>
          <DropdownMenuTrigger className={cn("inline-flex h-10 shrink-0 items-center gap-1.5 rounded-full border px-3.5 text-[13px] whitespace-nowrap md:h-8 md:px-3 md:text-[12.5px]", actor ? "border-primary/60 bg-primary/10" : "border-input bg-card text-muted-foreground")}>
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
      <div className="mt-3 rounded-[14px] border border-border bg-card px-4 md:mt-4 md:px-5">
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
