import { UploadTray } from "./files";
import { useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router";
import {
  ChefHat,
  Command as CommandIcon,
  CircleHelp,
  MessageCircle,
  Flame,
  FolderKanban,
  GitBranch,
  LogOut,
  Monitor,
  Moon,
  Pencil,
  Search,
  Sun,
  UserRound,
  Users,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { logout, useSession } from "@/lib/session";
import { useUi } from "@/lib/ui";
import { useLive } from "@/lib/live";
import { useAgents, useClaimedTasks, useInvalidateAll, useOpenQuestionCounts, useProjects, useV1, type AgentRow } from "@/lib/data";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useTheme } from "@/lib/theme";
import { lastView } from "@/lib/views";
import { Avatar, PersonName, StateDot, STATE_LABEL, cookState } from "./bits";
import { HandleDialog } from "./handle-dialog";
import { LogoMark, Wordmark } from "./brand";
import { CommandPalette } from "./command-palette";
import { BottomTabs, PhoneTopBar, TabletRail } from "./mobile-nav";
import { NewTaskDialog } from "./new-task";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

function navClass(active: boolean) {
  return cn(
    "group flex items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-[13px] text-sidebar-foreground/85 outline-offset-0 transition-colors hover:bg-sidebar-panel focus-visible:bg-sidebar-panel",
    active && "bg-sidebar-panel text-sidebar-foreground",
  );
}

function FireCard() {
  const relays = useV1<AgentRow[]>("/relays");
  const { status } = useLive();
  const now = useNow();
  const rows = relays.data ?? [];
  const lit = rows.filter((r) => !r.disabled && r.last_seen_at && now - new Date(r.last_seen_at).getTime() < 5 * 60_000);
  const newest = rows.map((r) => r.last_seen_at).filter(Boolean).sort().at(-1) ?? null;
  return (
    <NavLink
      to="/fires"
      className="block rounded-xl border border-sidebar-border bg-sidebar-panel p-3 transition-colors hover:border-sidebar-muted/40"
    >
      <div className="flex items-center gap-2">
        <Flame className={cn("size-4", lit.length ? "text-corn" : "text-sidebar-muted")} aria-hidden="true" />
        <span className="font-display text-[14px] text-sidebar-foreground">
          {lit.length ? (lit.length === 1 ? "Fire is lit" : `${lit.length} fires lit`) : "No fire lit"}
        </span>
        {lit.length > 0 && <span className="live-dot ml-auto size-2 rounded-full bg-corn" aria-hidden="true" />}
      </div>
      <p className="mt-1 text-[11.5px] leading-snug text-sidebar-muted">
        {rows.length === 0
          ? "No relay enrolled yet. A fire wakes your local cooks."
          : lit.length
            ? `${lit.map((l) => l.name).join(", ")} · seen ${fmtAgo(newest, now)}`
            : `Last relay seen ${fmtAgo(newest, now)}`}
      </p>
      <p className="mt-2 flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider text-sidebar-muted">
        <span className={cn("size-1.5 rounded-full", status === "live" ? "bg-success" : status === "offline" ? "bg-destructive" : "bg-sidebar-muted")} aria-hidden="true" />
        {status === "live" ? "kitchen live" : status === "offline" ? "reconnecting" : "connecting"}
      </p>
    </NavLink>
  );
}

function AccountMenu({ compact = false, side }: { compact?: boolean; side?: "top" | "right" | "bottom" }) {
  const me = useSession();
  const { theme, setTheme } = useTheme();
  const [renaming, setRenaming] = useState(false);
  return (
    <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Account menu"
          className={cn(
            "flex w-full items-center gap-2.5 rounded-lg p-1.5 text-left transition-colors hover:bg-sidebar-panel focus-visible:bg-sidebar-panel",
            compact && "size-12 w-auto justify-center rounded-xl",
          )}
        >
          <Avatar name={me.name} className="size-7 text-[13px]" />
          {!compact && (
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px] font-semibold text-sidebar-foreground">{me.display_name ?? me.name}</span>
              <span className="block truncate text-[11px] text-sidebar-muted">
                {me.display_name ? `@${me.name}` : me.is_admin ? "admin" : me.kind}
                {me.display_name && me.is_admin ? " · admin" : ""}
              </span>
            </span>
          )}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side={side ?? (compact ? "bottom" : "top")} className="w-56">
        <DropdownMenuLabel className="font-normal">
          <span className="block text-[11px] text-muted-foreground">Signed in as</span>
          <PersonName name={me.name} displayName={me.display_name} className="block break-words font-semibold" />
        </DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => setRenaming(true)}>
          <Pencil className="size-4" /> Change handle
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="label-caps py-1">Theme</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={theme} onValueChange={(v) => setTheme(v as typeof theme)}>
          <DropdownMenuRadioItem value="light">
            <Sun className="size-4" /> Light
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon className="size-4" /> Dark
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="system">
            <Monitor className="size-4" /> System
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => logout(me.csrf)} variant="destructive">
          <LogOut className="size-4" /> Log out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <HandleDialog open={renaming} onOpenChange={setRenaming} current={me.name} />
    </>
  );
}

function SidebarBody({ onNavigate }: { onNavigate?: () => void }) {
  const me = useSession();
  const projects = useProjects();
  const agents = useAgents();
  const claimed = useClaimedTasks(projects.data);
  const now = useNow();
  const loc = useLocation();
  const byCook = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of claimed.tasks) if (t.claimed_by && t.lease_live) m.set(t.claimed_by, t.id);
    return m;
  }, [claimed.tasks]);
  const asks = useOpenQuestionCounts(projects.data);
  const cooks = (agents.data ?? []).filter((a) => !a.disabled || a.busy > 0).slice(0, 7);

  return (
    <div className="flex h-full flex-col text-sidebar-foreground">
      <div className="flex items-center gap-2.5 px-4 pt-5 pb-4">
        <LogoMark className="size-7" />
        <Wordmark />
      </div>

      <nav aria-label="Main" className="scrollbar-thin flex-1 space-y-6 overflow-y-auto px-3 pb-4">
        <div>
          <div className="label-caps !text-sidebar-muted px-2.5 pb-1.5">Pots</div>
          <ul className="space-y-0.5">
            {projects.isLoading && [0, 1, 2].map((i) => <li key={i} className="mx-2.5 my-2 h-4 animate-pulse rounded bg-sidebar-panel" />)}
            {(projects.data ?? []).map((p) => {
              const active = loc.pathname.toLowerCase().startsWith(`/pots/${p.key.toLowerCase()}/`);
              const live = p.open + p.claimed + p.blocked;
              return (
                <li key={p.key}>
                  <NavLink
                    to={`/pots/${p.key}/${lastView(p.key)}`}
                    onClick={onNavigate}
                    aria-current={active ? "page" : undefined}
                    className={cn(
                      navClass(active),
                      active && "relative bg-corn/15 before:absolute before:inset-y-1.5 before:left-0 before:w-[3px] before:rounded-full before:bg-corn",
                    )}
                  >
                    <span className={cn("w-11 shrink-0 font-mono text-[11.5px] font-bold", active ? "text-corn" : "text-sidebar-muted")}>
                      {p.key}
                    </span>
                    <span className="min-w-0 flex-1 truncate">{p.name}</span>
                    {(asks[p.key] ?? 0) > 0 && (
                      <span className="inline-flex items-center gap-0.5 rounded-full bg-primary px-1.5 py-px font-mono text-[10.5px] font-bold text-primary-foreground">
                        <CircleHelp className="size-3" aria-hidden="true" />
                        <span aria-hidden="true">{asks[p.key]}</span>
                        <span className="sr-only">{plural(asks[p.key]!, "open question")}</span>
                      </span>
                    )}
                    {p.unread_pass > 0 && (
                      <span className="inline-flex items-center gap-0.5 rounded-full bg-primary px-1.5 py-px font-mono text-[10.5px] font-bold text-primary-foreground">
                        <MessageCircle className="size-3" aria-hidden="true" />
                        <span aria-hidden="true">{p.unread_pass}</span>
                        <span className="sr-only">{plural(p.unread_pass, "unread message")}</span>
                      </span>
                    )}
                    <span className="font-mono text-[11px] tabular-nums text-sidebar-muted" title={`${live} not served yet`}>
                      {live}
                    </span>
                  </NavLink>
                </li>
              );
            })}
            {projects.data?.length === 0 && (
              <li className="px-2.5 py-1 text-[12px] text-sidebar-muted">
                No pots yet.{me.is_admin ? " Start one under Pots below." : " Ask an admin to start one."}
              </li>
            )}
          </ul>
        </div>

        <div>
          <div className="flex items-center justify-between px-2.5 pb-1.5">
            <span className="label-caps !text-sidebar-muted">Cooks</span>
            <NavLink to="/cooks" onClick={onNavigate} className="text-[11px] text-sidebar-muted hover:text-sidebar-foreground">
              all
            </NavLink>
          </div>
          <ul className="space-y-0.5">
            {agents.isLoading && [0, 1].map((i) => <li key={i} className="mx-2.5 my-2 h-4 animate-pulse rounded bg-sidebar-panel" />)}
            {cooks.map((a) => {
              const st = cookState(a, now);
              const cur = byCook.get(a.name);
              return (
                <li key={a.name}>
                  <NavLink to={`/cooks/${a.name}`} onClick={onNavigate} className={navClass(false)}>
                    <StateDot state={st} />
                    <span className="min-w-0 flex-1 truncate">{a.name}</span>
                    <span className="max-w-24 truncate font-mono text-[10.5px] text-sidebar-muted">{cur ?? STATE_LABEL[st]}</span>
                  </NavLink>
                </li>
              );
            })}
            {agents.data?.length === 0 && <li className="px-2.5 py-1 text-[12px] text-sidebar-muted">No cooks yet.</li>}
          </ul>
        </div>

        <div>
          <div className="label-caps !text-sidebar-muted px-2.5 pb-1.5">Kitchen admin</div>
          <ul className="space-y-0.5">
            <SideLink to="/fires" icon={Flame} onNavigate={onNavigate}>
              Fires
            </SideLink>
            <SideLink to="/cooks" icon={ChefHat} onNavigate={onNavigate}>
              Cooks
            </SideLink>
            <SideLink to="/teams" icon={Users} onNavigate={onNavigate}>
              Teams
            </SideLink>
            {me.is_admin && (
              <>
                <SideLink to="/admin/pots" icon={FolderKanban} onNavigate={onNavigate}>
                  Pots
                </SideLink>
                <SideLink to="/admin/users" icon={UserRound} onNavigate={onNavigate}>
                  Users
                </SideLink>
                <SideLink to="/admin/forges" icon={GitBranch} onNavigate={onNavigate}>
                  Forges
                </SideLink>
              </>
            )}
          </ul>
        </div>
      </nav>

      <div className="space-y-3 border-t border-sidebar-border p-3">
        <FireCard />
        <AccountMenu />
      </div>
    </div>
  );
}

function SideLink({
  to,
  icon: Icon,
  children,
  onNavigate,
}: {
  to: string;
  icon: typeof Flame;
  children: React.ReactNode;
  onNavigate: (() => void) | undefined;
}) {
  return (
    <li>
      <NavLink to={to} onClick={onNavigate} className={({ isActive }) => navClass(isActive)}>
        <Icon className="size-4 text-sidebar-muted" aria-hidden="true" />
        {children}
      </NavLink>
    </li>
  );
}

export function AppShell() {
  const { setMenuOpen, setPaletteOpen } = useUi();
  const loc = useLocation();
  const invalidateAll = useInvalidateAll();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      // `r` refreshes, unless the person is typing or a modal dialog owns the keyboard
      if (e.key.toLowerCase() !== "r" || e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.closest("[role=combobox],[role=listbox]"))) return;
      if (document.querySelector("[data-slot=dialog-content],[data-slot=alert-dialog-content]")) return;
      e.preventDefault();
      void invalidateAll();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPaletteOpen, invalidateAll]);

  useEffect(() => setMenuOpen(false), [loc.pathname, setMenuOpen]);

  return (
    <div className="min-h-dvh bg-background">
      <a
        href="#main"
        className="sr-only z-[100] rounded-lg bg-primary px-3 py-2 text-primary-foreground focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to the kitchen
      </a>
      <aside className="fixed inset-y-0 left-0 z-30 hidden w-[256px] bg-sidebar lg:block" aria-label="Sidebar">
        <SidebarBody />
      </aside>

      <TabletRail account={<AccountMenu compact side="right" />} />

      <div className="md:pl-[72px] lg:pl-[256px]">
        <PhoneTopBar />
        <main id="main" tabIndex={-1} className="pb-[calc(4rem+env(safe-area-inset-bottom))] outline-none md:pb-0">
          <Outlet />
        </main>
      </div>
      <BottomTabs />
      <CommandPalette />
      <UploadTray />
      <NewTaskDialog />
    </div>
  );
}

export function PaletteButton({ className }: { className?: string }) {
  const { setPaletteOpen } = useUi();
  const mac = typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform);
  return (
    <button
      type="button"
      onClick={() => setPaletteOpen(true)}
      className={cn(
        "hidden h-9 items-center gap-2 rounded-lg border border-input bg-card px-3 text-[13px] text-muted-foreground transition-colors hover:border-foreground/30 sm:inline-flex",
        className,
      )}
    >
      <Search className="size-3.5" aria-hidden="true" />
      <span className="w-28 text-left">Search or jump</span>
      <kbd className="inline-flex items-center gap-0.5 rounded border border-border bg-muted px-1.5 py-px font-mono text-[10.5px]">
        {mac ? <CommandIcon className="size-2.5" aria-hidden="true" /> : <span>Ctrl</span>}K
      </kbd>
    </button>
  );
}
