import { useCallback, useMemo } from "react";
import { useLocation, useNavigate } from "react-router";
import type { MenuView, ProjectSummary, TaskView } from "@corntchap/api";
import { BookMarked, ChefHat, GitBranch, MessageCircleQuestion, Soup, Flame, FolderKanban, LogOut, Monitor, Moon, Plus, Sun, UserRound, Users } from "lucide-react";
import { useUi } from "@/lib/ui";
import { useProjects } from "@/lib/data";
import { useMenusSource, usePotsSource, useTasksSource } from "@/lib/sources";
import { useMappedSource, useStaticSource } from "@/lib/listbox";
import { useStoreValue } from "@/lib/use-store-value";
import { useTheme } from "@/lib/theme";
import { canOrchestrate, isSuperAdmin, logout, useSession } from "@/lib/session";
import { lastView } from "@/lib/views";
import { useOpenTask } from "@/lib/nav";
import { BUCKETS, bucketOf } from "@/lib/format";
import { VirtualCommand, type CommandItem, type CommandSection } from "./ui/virtual-command";

/**
 * Search or jump: the pages and actions (narrowed here), and the pots, the open menus and the tasks of the pot you
 * are in (narrowed by the server as you type, a page at a time).
 */
export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, openNewTask } = useUi();
  const navigate = useNavigate();
  const loc = useLocation();
  const me = useSession();
  const { setTheme } = useTheme();
  const openTask = useOpenTask();
  const projects = useProjects();
  const query = useStoreValue("");
  const pot = /^\/pots\/([^/]+)/.exec(loc.pathname)?.[1]?.toUpperCase();
  const asks = projects.data?.find((p) => p.key === pot)?.open_questions ?? 0;
  const anyPot = (projects.data ?? []).length > 0;

  const potRows = usePotsSource({ q: query.value, enabled: paletteOpen });
  const menuRows = useMenusSource(pot ?? "", { q: query.value, enabled: !!pot && paletteOpen });
  const taskRows = useTasksSource(pot ?? "", { q: query.value, enabled: !!pot && paletteOpen });

  const go = useCallback((to: string) => () => void navigate(to), [navigate]);

  const potItem = useCallback(
    (p: ProjectSummary): CommandItem => ({ id: p.key, label: p.name, hint: p.key, icon: <FolderKanban />, onSelect: go(`/pots/${p.key}/${lastView(p.key)}`) }),
    [go],
  );
  const menuItem = useCallback(
    (m: MenuView): CommandItem => ({
      id: m.key,
      label: m.name,
      hint: `${m.done}/${m.total}`,
      icon: <Soup />,
      onSelect: go(`/pots/${pot}/board?menu=${encodeURIComponent(m.key)}`),
    }),
    [go, pot],
  );
  const taskItem = useCallback(
    (t: TaskView): CommandItem => ({ id: t.id, label: t.title, hint: `${t.id} · ${BUCKETS.find((b) => b.id === bucketOf(t))?.label ?? ""}`, onSelect: () => openTask(t.id) }),
    [openTask],
  );
  const pots = useMappedSource(potRows, potItem);
  const menus = useMappedSource(menuRows, menuItem);
  const tasks = useMappedSource(taskRows, taskItem);

  const doItems = useMemo<CommandItem[]>(
    () => [
      ...(pot ? [{ id: "pantry", label: `Search the pantry in ${pot}`, keywords: ["memory", "memories", "recall"], icon: <BookMarked />, onSelect: () => void navigate(`/pots/${pot}/pantry`, { state: { focus: true } }) }] : []),
      ...(pot
        ? [
            { id: "questions", label: `Questions in ${pot}`, keywords: ["ask", "answer", "decisions", "chef"], icon: <MessageCircleQuestion />, hint: asks > 0 ? `${asks} open` : undefined, onSelect: go(`/pots/${pot}/questions`) },
            { id: "new-menu", label: `New menu in ${pot}`, keywords: ["milestone", "create", "add"], icon: <Plus />, onSelect: go(`/pots/${pot}/menus?new=1`) },
            { id: "all-menus", label: `All menus in ${pot}`, keywords: ["switch menu", "board"], icon: <Soup />, onSelect: go(`/pots/${pot}/board`) },
          ]
        : []),
      ...(anyPot ? [{ id: "new-task", label: `New task${pot ? ` in ${pot}` : ""}`, keywords: ["add", "create"], icon: <Plus />, onSelect: () => openNewTask(pot) }] : []),
    ],
    [pot, asks, anyPot, navigate, go, openNewTask],
  );
  const goItems = useMemo<CommandItem[]>(
    () => [
      { id: "cooks", label: "Cooks", keywords: ["agents"], icon: <ChefHat />, onSelect: go("/cooks") },
      { id: "fires", label: "Fires", keywords: ["relays"], icon: <Flame />, onSelect: go("/fires") },
      { id: "teams", label: "Teams", keywords: ["chefs"], icon: <Users />, onSelect: go("/teams") },
      ...(canOrchestrate(me)
        ? [
            { id: "admin-pots", label: "Pots", keywords: ["admin", "projects"], icon: <FolderKanban />, onSelect: go("/admin/pots") },
            { id: "new-team", label: "New team", keywords: ["create team"], icon: <Users />, onSelect: go("/teams") },
          ]
        : []),
      ...(isSuperAdmin(me)
        ? [
            { id: "admin-users", label: "Users (admin)", keywords: ["admin"], icon: <UserRound />, onSelect: go("/admin/users") },
            { id: "admin-forges", label: "Forges (admin)", keywords: ["admin", "github"], icon: <GitBranch />, onSelect: go("/admin/forges") },
          ]
        : []),
    ],
    [me, go],
  );
  const settingItems = useMemo<CommandItem[]>(
    () => [
      { id: "light", label: "Light theme", keywords: ["theme"], icon: <Sun />, onSelect: () => setTheme("light") },
      { id: "dark", label: "Dark theme", keywords: ["theme"], icon: <Moon />, onSelect: () => setTheme("dark") },
      { id: "system", label: "Follow the system theme", keywords: ["theme"], icon: <Monitor />, onSelect: () => setTheme("system") },
      { id: "logout", label: "Log out", keywords: ["sign out"], icon: <LogOut />, onSelect: () => logout(me.csrf) },
    ],
    [setTheme, me.csrf],
  );
  const doSource = useStaticSource(doItems);
  const goSource = useStaticSource(goItems);
  const settingSource = useStaticSource(settingItems);

  const sections = useMemo<CommandSection[]>(
    () => [
      { id: "do", heading: "Do", source: doSource, filter: "client" },
      ...(pot ? [{ id: "menus", heading: `Switch menu in ${pot}`, source: menus, filter: "server" as const }] : []),
      { id: "pots", heading: "Pots", source: pots, filter: "server" },
      ...(pot ? [{ id: "tasks", heading: `Tasks in ${pot}`, source: tasks, filter: "server" as const }] : []),
      { id: "go", heading: "Go to", source: goSource, filter: "client" },
      { id: "settings", heading: "Settings", source: settingSource, filter: "client" },
    ],
    [pot, doSource, menus, pots, tasks, goSource, settingSource],
  );

  return (
    <VirtualCommand
      open={paletteOpen}
      onOpenChange={(o) => {
        setPaletteOpen(o);
        if (!o) query.set("");
      }}
      title="Search or jump"
      description="Jump to a pot, a task or a page, or start something new."
      placeholder="Jump to a pot, a task, a page…"
      query={query.value}
      onQuery={query.set}
      sections={sections}
      empty="Nothing in the pot matches that."
    />
  );
}
