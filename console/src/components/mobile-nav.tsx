import { useMemo, useState, type ComponentType, type ReactNode } from "react";
import { NavLink, useLocation, useNavigate } from "react-router";
import {
  Activity,
  BookOpen,
  ChefHat,
  ChevronDown,
  CircleHelp,
  Files,
  Flame,
  FolderKanban,
  Info,
  LayoutGrid,
  LogOut,
  MessageCircle,
  Monitor,
  Moon,
  MoreHorizontal,
  Pencil,
  Plus,
  ScrollText,
  Search,
  Sun,
  UserRound,
  Users,
} from "lucide-react";
import type { ProjectSummary } from "@corntchap/api";
import { cn } from "@/lib/utils";
import { logout, useSession } from "@/lib/session";
import { useUi } from "@/lib/ui";
import { useLive } from "@/lib/live";
import { useProjects, useV1, type AgentRow } from "@/lib/data";
import { useTheme } from "@/lib/theme";
import { lastPot, lastView } from "@/lib/views";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { Avatar } from "./bits";
import { LogoMark } from "./brand";
import { HandleDialog } from "./handle-dialog";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

type Icon = ComponentType<{ className?: string; "aria-hidden"?: boolean | "true" }>;

/** The pot the phone tabs follow: the one in the address, else the last one opened, else the first. */
export function useCurrentPot(): ProjectSummary | null {
  const loc = useLocation();
  const projects = useProjects();
  const inPath = /^\/pots\/([^/]+)/i.exec(loc.pathname)?.[1]?.toUpperCase();
  const list = projects.data ?? [];
  return list.find((p) => p.key === inPath) ?? list.find((p) => p.key === lastPot()) ?? list[0] ?? null;
}

function Badge({ n, label }: { n: number; label: string }) {
  if (n <= 0) return null;
  return (
    <span className="absolute -top-1 left-1/2 ml-1.5 inline-flex min-w-4 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10px] leading-4 font-bold text-primary-foreground">
      <span aria-hidden="true">{n > 99 ? "99+" : n}</span>
      <span className="sr-only">{plural(n, label)}</span>
    </span>
  );
}

function TabLink({ to, icon: I, label, active, badge }: { to: string; icon: Icon; label: string; active: boolean; badge?: ReactNode }) {
  return (
    <NavLink
      to={to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex min-h-14 flex-1 flex-col items-center justify-center gap-0.5 text-[10.5px] font-medium text-muted-foreground transition-colors active:bg-accent/60",
        active && "text-foreground",
      )}
    >
      <span className={cn("relative flex h-7 w-12 items-center justify-center rounded-full transition-colors", active && "bg-corn/25")}>
        <I className="size-[19px]" aria-hidden="true" />
        {badge}
      </span>
      {label}
    </NavLink>
  );
}

/**
 * Phones: the bottom tab bar, scoped to the current pot (Board, Pass, Questions), then Cooks and More.
 * Everything else (other pot tabs, admin, theme, account) lives in the More sheet.
 */
export function BottomTabs() {
  const pot = useCurrentPot();
  const loc = useLocation();
  const [more, setMore] = useState(false);
  const path = loc.pathname.toLowerCase();
  const base = pot ? `/pots/${pot.key.toLowerCase()}` : "";
  const on = (seg: string) => !!pot && path === `${base}/${seg}`;
  const board = !!pot && (on("board") || path.startsWith(`${base}/v/`));
  const cooks = path === "/cooks" || path.startsWith("/cooks/");
  const known = board || on("pass") || on("questions") || cooks;
  const potTo = (seg: string) => (pot ? `/pots/${pot.key}/${seg}` : "/");
  return (
    <>
      <nav
        aria-label="Main"
        className="fixed inset-x-0 bottom-0 z-40 border-t border-border bg-background/95 pb-[env(safe-area-inset-bottom)] backdrop-blur-md supports-[backdrop-filter]:bg-background/80 md:hidden"
      >
        <div className="mx-auto flex max-w-lg items-stretch">
          <TabLink to={pot ? `/pots/${pot.key}/${board ? "board" : lastView(pot.key).startsWith("v/") ? lastView(pot.key) : "board"}` : "/"} icon={LayoutGrid} label="Board" active={board} />
          <TabLink to={potTo("pass")} icon={MessageCircle} label="Pass" active={on("pass")} badge={<Badge n={pot?.unread_pass ?? 0} label="unread message" />} />
          <TabLink to={potTo("questions")} icon={CircleHelp} label="Questions" active={on("questions")} badge={<Badge n={pot?.open_questions ?? 0} label="open question" />} />
          <TabLink to="/cooks" icon={ChefHat} label="Cooks" active={cooks} />
          <button
            type="button"
            onClick={() => setMore(true)}
            aria-haspopup="dialog"
            className={cn(
              "relative flex min-h-14 flex-1 flex-col items-center justify-center gap-0.5 text-[10.5px] font-medium text-muted-foreground active:bg-accent/60",
              !known && "text-foreground",
            )}
          >
            <span className={cn("flex h-7 w-12 items-center justify-center rounded-full", !known && "bg-corn/25")}>
              <MoreHorizontal className="size-[19px]" aria-hidden="true" />
            </span>
            More
          </button>
        </div>
      </nav>
      <MoreSheet open={more} onOpenChange={setMore} pot={pot} />
    </>
  );
}

function SheetRow({ to, icon: I, children, onPick, hint }: { to: string; icon: Icon; children: ReactNode; onPick: () => void; hint?: ReactNode }) {
  return (
    <NavLink
      to={to}
      onClick={onPick}
      className={({ isActive }) =>
        cn(
          "flex min-h-12 items-center gap-3 rounded-xl px-3 text-[15px] transition-colors active:bg-accent",
          isActive && "bg-accent font-medium",
        )
      }
    >
      <I className="size-[18px] text-muted-foreground" aria-hidden="true" />
      <span className="flex-1">{children}</span>
      {hint && <span className="font-mono text-[11px] text-muted-foreground">{hint}</span>}
    </NavLink>
  );
}

function ThemeSwitch() {
  const { theme, setTheme } = useTheme();
  const opts = [
    { v: "light", label: "Light", icon: Sun },
    { v: "dark", label: "Dark", icon: Moon },
    { v: "system", label: "System", icon: Monitor },
  ] as const;
  return (
    <div role="radiogroup" aria-label="Theme" className="grid grid-cols-3 gap-1 rounded-xl bg-muted p-1">
      {opts.map(({ v, label, icon: I }) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={theme === v}
          onClick={() => setTheme(v)}
          className={cn(
            "flex min-h-10 items-center justify-center gap-1.5 rounded-lg text-[13px] text-muted-foreground transition-colors",
            theme === v && "bg-card text-foreground shadow-sm",
          )}
        >
          <I className="size-4" aria-hidden="true" /> {label}
        </button>
      ))}
    </div>
  );
}

/** The relays' state in one line: whether a fire is lit to wake local cooks. */
function FireLine({ onPick }: { onPick: () => void }) {
  const relays = useV1<AgentRow[]>("/relays");
  const { status } = useLive();
  const now = useNow();
  const rows = relays.data ?? [];
  const lit = rows.filter((r) => !r.disabled && r.last_seen_at && now - new Date(r.last_seen_at).getTime() < 5 * 60_000);
  const newest = rows.map((r) => r.last_seen_at).filter(Boolean).sort().at(-1) ?? null;
  return (
    <NavLink to="/fires" onClick={onPick} className="flex items-center gap-3 rounded-xl border border-border bg-card px-3 py-2.5">
      <Flame className={cn("size-5", lit.length ? "text-corn" : "text-muted-foreground")} aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="block font-display text-[15px]">{lit.length ? (lit.length === 1 ? "Fire is lit" : `${lit.length} fires lit`) : "No fire lit"}</span>
        <span className="block truncate text-[12px] text-muted-foreground">
          {rows.length === 0 ? "No relay enrolled yet" : `Last relay seen ${fmtAgo(newest, now)}`}
        </span>
      </span>
      <span className="flex items-center gap-1.5 font-mono text-[10px] tracking-wider text-muted-foreground uppercase">
        <span className={cn("size-1.5 rounded-full", status === "live" ? "bg-success" : status === "offline" ? "bg-destructive" : "bg-muted-foreground")} aria-hidden="true" />
        {status === "live" ? "live" : status === "offline" ? "offline" : "…"}
      </span>
    </NavLink>
  );
}

export function MoreSheet({ open, onOpenChange, pot }: { open: boolean; onOpenChange: (v: boolean) => void; pot: ProjectSummary | null }) {
  const me = useSession();
  const [renaming, setRenaming] = useState(false);
  const close = () => onOpenChange(false);
  return (
    <>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent side="bottom" className="gap-0 px-3">
          <SheetTitle className="sr-only">More</SheetTitle>
          <SheetDescription className="sr-only">Other pot tabs, the kitchen admin pages, theme and account</SheetDescription>
          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto overscroll-contain pt-2 pb-2">
            {pot && (
              <section>
                <h2 className="label-caps px-3 pb-1">{pot.name.toUpperCase() === pot.key ? pot.key : `${pot.key} · ${pot.name}`}</h2>
                <SheetRow to={`/pots/${pot.key}/activity`} icon={Activity} onPick={close}>Activity</SheetRow>
                <SheetRow to={`/pots/${pot.key}/pantry`} icon={BookOpen} onPick={close}>Pantry</SheetRow>
                <SheetRow to={`/pots/${pot.key}/menus`} icon={ScrollText} onPick={close}>Menus</SheetRow>
                <SheetRow to={`/pots/${pot.key}/files`} icon={Files} onPick={close}>Files</SheetRow>
                <SheetRow to={`/pots/${pot.key}/about`} icon={Info} onPick={close}>About this pot</SheetRow>
              </section>
            )}
            <section>
              <h2 className="label-caps px-3 pb-1">Kitchen</h2>
              <SheetRow to="/fires" icon={Flame} onPick={close}>Fires</SheetRow>
              <SheetRow to="/teams" icon={Users} onPick={close}>Teams</SheetRow>
              {me.is_admin && (
                <>
                  <SheetRow to="/admin/pots" icon={FolderKanban} onPick={close}>Pots</SheetRow>
                  <SheetRow to="/admin/users" icon={UserRound} onPick={close}>Users</SheetRow>
                </>
              )}
            </section>
            <section className="space-y-3 px-1">
              <FireLine onPick={close} />
              <ThemeSwitch />
            </section>
            <section className="flex items-center gap-3 rounded-xl border border-border bg-card p-3">
              <Avatar name={me.name} className="size-9 text-[15px]" />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-semibold">{me.display_name ?? me.name}</span>
                <span className="block truncate text-[12px] text-muted-foreground">
                  @{me.name}
                  {me.is_admin ? " · admin" : ""}
                </span>
              </span>
              <button type="button" onClick={() => setRenaming(true)} className="flex size-10 items-center justify-center rounded-lg text-muted-foreground active:bg-accent" aria-label="Change handle">
                <Pencil className="size-4" />
              </button>
              <button type="button" onClick={() => logout(me.csrf)} className="flex size-10 items-center justify-center rounded-lg text-destructive active:bg-accent" aria-label="Log out">
                <LogOut className="size-4" />
              </button>
            </section>
          </div>
        </SheetContent>
      </Sheet>
      <HandleDialog open={renaming} onOpenChange={setRenaming} current={me.name} />
    </>
  );
}

/** Pick a pot: a list with each pot's live counts. */
export function PotSheet({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const projects = useProjects();
  const current = useCurrentPot();
  const navigate = useNavigate();
  const me = useSession();
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="bottom" className="gap-0 px-3">
        <SheetTitle className="px-3 pt-1 font-display text-[20px]">Pots</SheetTitle>
        <SheetDescription className="px-3 pb-3 text-[13px] text-muted-foreground">Switch the pot your tabs follow.</SheetDescription>
        <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto overscroll-contain pb-2">
          {(projects.data ?? []).map((p) => {
            const live = p.ready + p.claimed + p.blocked;
            const active = p.key === current?.key;
            return (
              <li key={p.key}>
                <button
                  type="button"
                  onClick={() => {
                    onOpenChange(false);
                    navigate(`/pots/${p.key}/${lastView(p.key)}`);
                  }}
                  aria-current={active ? "true" : undefined}
                  className={cn("flex min-h-14 w-full items-center gap-3 rounded-xl px-3 text-left active:bg-accent", active && "bg-corn/15")}
                >
                  <span className={cn("w-12 shrink-0 rounded-md px-1.5 py-0.5 text-center font-mono text-[12px] font-bold", active ? "bg-corn text-corn-foreground" : "bg-muted")}>
                    {p.key}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[15px] font-medium">{p.name}</span>
                    <span className="block truncate text-[12px] text-muted-foreground">
                      {p.claimed} on the fire · {p.ready} ready{p.team ? ` · ${p.team}` : ""}
                    </span>
                  </span>
                  {p.open_questions > 0 && (
                    <span className="inline-flex items-center gap-0.5 rounded-full bg-primary px-1.5 font-mono text-[11px] font-bold text-primary-foreground">
                      <CircleHelp className="size-3" aria-hidden="true" /> {p.open_questions}
                    </span>
                  )}
                  <span className="font-mono text-[12px] text-muted-foreground tabular-nums" aria-label={`${live} not served yet`}>
                    {live}
                  </span>
                </button>
              </li>
            );
          })}
          {projects.data?.length === 0 && (
            <li className="px-3 py-4 text-[14px] text-muted-foreground">No pots yet.{me.is_admin ? " Start one under More, Pots." : " Ask an admin to start one."}</li>
          )}
        </ul>
      </SheetContent>
    </Sheet>
  );
}

/** Phones: the top bar with the pot switcher, search and New task. */
export function PhoneTopBar() {
  const pot = useCurrentPot();
  const { setPaletteOpen, openNewTask } = useUi();
  const [pots, setPots] = useState(false);
  return (
    <>
      <header className="sticky top-0 z-30 border-b border-border bg-background/95 pt-[env(safe-area-inset-top)] backdrop-blur-md supports-[backdrop-filter]:bg-background/80 md:hidden">
        <div className="flex h-14 items-center gap-1 px-2">
          <button
            type="button"
            onClick={() => setPots(true)}
            aria-haspopup="dialog"
            aria-label={pot ? `Pot ${pot.name}, switch pot` : "Choose a pot"}
            className="flex min-h-11 min-w-0 flex-1 items-center gap-2 rounded-xl px-2 text-left active:bg-accent"
          >
            <LogoMark className="size-7 shrink-0" />
            {pot ? (
              <>
                <span className="shrink-0 rounded-md bg-corn px-1.5 py-0.5 font-mono text-[11px] font-bold text-corn-foreground">{pot.key}</span>
                <span className="min-w-0 truncate font-display text-[18px] font-extrabold">{pot.name}</span>
              </>
            ) : (
              <span className="font-display text-[18px] font-extrabold">corntchap</span>
            )}
            <ChevronDown className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          </button>
          <button type="button" onClick={() => setPaletteOpen(true)} aria-label="Search and jump" className="flex size-11 items-center justify-center rounded-xl active:bg-accent">
            <Search className="size-5" />
          </button>
          <button
            type="button"
            onClick={() => openNewTask(pot?.key)}
            aria-label="New task"
            className="flex size-11 items-center justify-center rounded-xl text-primary active:bg-accent"
          >
            <Plus className="size-6" />
          </button>
        </div>
      </header>
      <PotSheet open={pots} onOpenChange={setPots} />
    </>
  );
}

function RailLink({ to, label, children, active, badge }: { to: string; label: string; children: ReactNode; active?: boolean; badge?: number }) {
  const loc = useLocation();
  // a plain string: the tooltip's Slot cannot merge NavLink's className function (the styles were lost)
  const on = active ?? (loc.pathname === to || loc.pathname.startsWith(`${to}/`));
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <NavLink
          to={to}
          aria-label={label}
          aria-current={on ? "page" : undefined}
          className={cn(
            "relative flex size-12 items-center justify-center rounded-xl text-sidebar-muted transition-colors hover:bg-sidebar-panel hover:text-sidebar-foreground",
            on && "bg-sidebar-panel text-sidebar-foreground",
          )}
        >
          {children}
          {!!badge && badge > 0 && <span className="absolute top-1.5 right-1.5 size-2 rounded-full bg-primary" aria-hidden="true" />}
        </NavLink>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

/** Tablets: an icon rail with the pots (by key) and the kitchen pages; account and search below. */
export function TabletRail({ account }: { account: ReactNode }) {
  const projects = useProjects();
  const loc = useLocation();
  const me = useSession();
  const { setPaletteOpen } = useUi();
  const path = loc.pathname.toLowerCase();
  const pots = useMemo(() => projects.data ?? [], [projects.data]);
  return (
    <aside aria-label="Sidebar" className="fixed inset-y-0 left-0 z-30 hidden w-[72px] flex-col items-center bg-sidebar pt-[max(0.75rem,env(safe-area-inset-top))] pb-[max(0.75rem,env(safe-area-inset-bottom))] md:flex lg:hidden">
      <NavLink to="/" aria-label="corntchap home" className="mb-3 flex size-12 items-center justify-center">
        <LogoMark className="size-8" />
      </NavLink>
      <nav aria-label="Pots" className="scrollbar-none flex w-full flex-1 flex-col items-center gap-1 overflow-y-auto">
        {pots.map((p) => {
          const active = path.startsWith(`/pots/${p.key.toLowerCase()}/`);
          return (
            <RailLink key={p.key} to={`/pots/${p.key}/${lastView(p.key)}`} label={`${p.name} (${p.key})`} active={active} badge={p.open_questions + p.unread_pass}>
              <span className={cn("font-mono text-[11.5px] font-bold", active && "text-corn")}>{p.key.slice(0, 4)}</span>
              {active && <span className="absolute inset-y-2 -left-3 w-[3px] rounded-full bg-corn" aria-hidden="true" />}
            </RailLink>
          );
        })}
        <div className="my-2 h-px w-8 bg-sidebar-border" aria-hidden="true" />
        <RailLink to="/cooks" label="Cooks">
          <ChefHat className="size-5" aria-hidden="true" />
        </RailLink>
        <RailLink to="/fires" label="Fires">
          <Flame className="size-5" aria-hidden="true" />
        </RailLink>
        <RailLink to="/teams" label="Teams">
          <Users className="size-5" aria-hidden="true" />
        </RailLink>
        {me.is_admin && (
          <>
            <RailLink to="/admin/pots" label="Pots (admin)">
              <FolderKanban className="size-5" aria-hidden="true" />
            </RailLink>
            <RailLink to="/admin/users" label="Users (admin)">
              <UserRound className="size-5" aria-hidden="true" />
            </RailLink>
          </>
        )}
      </nav>
      <div className="mt-2 flex flex-col items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <button type="button" onClick={() => setPaletteOpen(true)} aria-label="Search and jump" className="flex size-12 items-center justify-center rounded-xl text-sidebar-muted hover:bg-sidebar-panel hover:text-sidebar-foreground">
              <Search className="size-5" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">Search and jump</TooltipContent>
        </Tooltip>
        {account}
      </div>
    </aside>
  );
}
