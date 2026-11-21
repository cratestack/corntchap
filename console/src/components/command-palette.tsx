import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router";
import { BookMarked, ChefHat, Flame, FolderKanban, LogOut, Monitor, Moon, Plus, Sun, UserRound, Users } from "lucide-react";
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
import { useProc, useProjects } from "@/lib/data";
import { useTheme } from "@/lib/theme";
import { logout, useSession } from "@/lib/session";
import { lastView } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
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
    >
      <CommandInput placeholder="Jump to a pot, a task, a page…" />
      <CommandList className="max-h-[min(420px,60vh)]">
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
          <CommandItem value="new task add create" onSelect={run(() => openNewTask(pot))}>
            <Plus /> New task{pot ? ` in ${pot}` : ""}
          </CommandItem>
        </CommandGroup>
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
          {me.is_admin && (
            <>
              <CommandItem value="admin pots projects" onSelect={run(() => navigate("/admin/pots"))}>
                <FolderKanban /> Pots (admin)
              </CommandItem>
              <CommandItem value="admin teams" onSelect={run(() => navigate("/admin/teams"))}>
                <Users /> Teams (admin)
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
