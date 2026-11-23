import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router";
import { BookMarked, ChefHat, MessageCircleQuestion, Soup, Flame, FolderKanban, LogOut, Monitor, Moon, Plus, Sun, UserRound, Users } from "lucide-react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { useUi } from "@/lib/ui";
import { useProc, useProjects, useQuestions } from "@/lib/data";
import { useTheme } from "@/lib/theme";
import { logout, useSession } from "@/lib/session";
import { lastView } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
import { useMenus } from "./menus";
import { BUCKETS, bucketOf } from "@/lib/format";

export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, openNewTask } = useUi();
  const navigate = useNavigate();
  const loc = useLocation();
  const me = useSession();
  const { setTheme } = useTheme();
  const openTask = useOpenTask();
  const projects = useProjects();
  const pot = /^\/pots\/([^/]+)/.exec(loc.pathname)?.[1]?.toUpperCase();
  const tasks = useProc("projectTasks", { project: pot ?? "" }, { enabled: !!pot && paletteOpen });

  const menus = useMenus(pot ?? "", !!pot && paletteOpen);
  const asks = useQuestions(pot ?? "", "open", !!pot && paletteOpen).data?.length ?? 0;

  const run = (fn: () => void) => () => {
    setPaletteOpen(false);
    fn();
  };

  const taskItems = useMemo(() => (tasks.data ?? []).slice(0, 80), [tasks.data]);

  return (
    <CommandDialog
      open={paletteOpen}
      onOpenChange={setPaletteOpen}
      title="Search or jump"
      description="Jump to a pot, a task or a page, or start something new."
      className="max-sm:top-0 max-sm:h-dvh max-sm:max-h-dvh max-sm:rounded-none max-sm:border-0 max-sm:pt-[env(safe-area-inset-top)] max-sm:pb-[env(safe-area-inset-bottom)]"
    >
      <CommandInput placeholder="Jump to a pot, a task, a page…" className="max-sm:pr-8 max-sm:text-base" />
      <CommandList className="max-h-[min(420px,60vh)] max-sm:max-h-none max-sm:flex-1 max-sm:pb-4 max-sm:[&_[cmdk-item]]:min-h-12 max-sm:[&_[cmdk-item]]:text-[15px]">
        <CommandEmpty>Nothing in the pot matches that.</CommandEmpty>
        <CommandGroup heading="Do">
          {pot && (
            <CommandItem
              value="search the pantry memory memories recall"
              onSelect={run(() => navigate(`/pots/${pot}/pantry`, { state: { focus: true } }))}
            >
              <BookMarked /> Search the pantry in {pot}
            </CommandItem>
          )}
          {pot && (
            <CommandItem value="questions for the chef ask answer decisions" onSelect={run(() => navigate(`/pots/${pot}/questions`))}>
              <MessageCircleQuestion /> Questions in {pot}
              {asks > 0 && <span className="ml-auto font-mono text-[11px] text-muted-foreground">{asks} open</span>}
            </CommandItem>
          )}
          {pot && (
            <CommandItem value="new menu milestone create add" onSelect={run(() => navigate(`/pots/${pot}/menus?new=1`))}>
              <Plus /> New menu in {pot}
            </CommandItem>
          )}
          {(projects.data ?? []).length > 0 && (
            <CommandItem value="new task add create" onSelect={run(() => openNewTask(pot))}>
              <Plus /> New task{pot ? ` in ${pot}` : ""}
            </CommandItem>
          )}
        </CommandGroup>
        {pot && menus.open.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading={`Switch menu in ${pot}`}>
              <CommandItem value="switch menu all menus" onSelect={run(() => navigate(`/pots/${pot}/board`))}>
                <Soup /> All menus
              </CommandItem>
              {menus.open.map((m) => (
                <CommandItem key={m.key} value={`switch menu ${m.key} ${m.name}`} onSelect={run(() => navigate(`/pots/${pot}/board?menu=${encodeURIComponent(m.key)}`))}>
                  <Soup /> {m.name}
                  <span className="ml-auto font-mono text-[11px] text-muted-foreground">
                    {m.done}/{m.total}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}
        <CommandSeparator />
        <CommandGroup heading="Pots">
          {(projects.data ?? []).map((p) => (
            <CommandItem key={p.key} value={`pot ${p.key} ${p.name}`} onSelect={run(() => navigate(`/pots/${p.key}/${lastView(p.key)}`))}>
              <FolderKanban />
              <span className="font-mono text-[12px] font-bold">{p.key}</span>
              <span className="text-muted-foreground">{p.name}</span>
            </CommandItem>
          ))}
        </CommandGroup>
        {taskItems.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading={`Tasks in ${pot}`}>
              {taskItems.map((t) => (
                <CommandItem key={t.id} value={`task ${t.id} ${t.title} ${t.labels.join(" ")}`} onSelect={run(() => openTask(t.id))}>
                  <span className="w-14 shrink-0 font-mono text-[12px] font-bold">{t.id}</span>
                  <span className="min-w-0 flex-1 truncate">{t.title}</span>
                  <span className="text-[11px] text-muted-foreground">{BUCKETS.find((b) => b.id === bucketOf(t))?.label}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}
        <CommandSeparator />
        <CommandGroup heading="Go to">
          <CommandItem value="cooks agents" onSelect={run(() => navigate("/cooks"))}>
            <ChefHat /> Cooks
          </CommandItem>
          <CommandItem value="fires relays" onSelect={run(() => navigate("/fires"))}>
            <Flame /> Fires
          </CommandItem>
          <CommandItem value="teams chefs" onSelect={run(() => navigate("/teams"))}>
            <Users /> Teams
          </CommandItem>
          {me.is_admin && (
            <>
              <CommandItem value="admin pots projects" onSelect={run(() => navigate("/admin/pots"))}>
                <FolderKanban /> Pots (admin)
              </CommandItem>
              <CommandItem value="admin users" onSelect={run(() => navigate("/admin/users"))}>
                <UserRound /> Users (admin)
              </CommandItem>
            </>
          )}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Settings">
          <CommandItem value="theme light" onSelect={run(() => setTheme("light"))}>
            <Sun /> Light theme
          </CommandItem>
          <CommandItem value="theme dark" onSelect={run(() => setTheme("dark"))}>
            <Moon /> Dark theme
          </CommandItem>
          <CommandItem value="theme system" onSelect={run(() => setTheme("system"))}>
            <Monitor /> Follow the system theme
          </CommandItem>
          <CommandItem value="log out sign out" onSelect={run(() => logout(me.csrf))}>
            <LogOut /> Log out
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
