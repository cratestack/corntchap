import { useEffect, useState, type FormEvent } from "react";
import { Link } from "react-router";
import type { MenuView } from "@corntchap/api";
import { ChevronRight, Loader2, MoreHorizontal, PartyPopper, Pencil, Plus, RotateCcw, Soup } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProcCall } from "@/lib/data";
import { useMenusSource } from "@/lib/sources";
import { useUrlFilters } from "@/lib/url-filters";
import { errorMessage } from "@/lib/api";
import { plural } from "@/lib/format";
import type { ListSource } from "@/lib/listbox";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
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
import { EmptyState, RowSkeleton } from "./bits";
import { FilterBar } from "./filter-bar";
import { listFooter, loadMore, type MoreMode } from "./list-footer";
import { VirtualList } from "./ui/virtual-list";

/**
 * One menu (milestone) of a pot by its key: the paged read narrowed by the key (it matches key and name), and the
 * row that is exactly it, pulling further pages while it is not among them. `menu` is undefined while that runs
 * and for a key that is not a menu of the pot.
 */
export function useMenu(pot: string, key: string | undefined) {
  const on = !!pot && !!key && key !== "-";
  const source = useMenusSource(pot, { q: key ?? "", status: "all", enabled: on });
  const menu = on ? source.items.find((m) => m.key === key) : undefined;
  const { hasNextPage, isFetchingNextPage, pending, fetchNextPage } = source;
  useEffect(() => {
    if (on && !menu && hasNextPage && !isFetchingNextPage && !pending) fetchNextPage();
  }, [on, menu, hasNextPage, isFetchingNextPage, pending, fetchNextPage]);
  return { menu, isLoading: on && !menu && (source.isLoading || !!hasNextPage) };
}

/** A menu as a facet or picker option: its key, its name and how far along it is. */
export const menuOption = (m: MenuView) => ({ value: m.key, label: m.name, hint: m.status === "served" ? "served" : `${m.done}/${m.total}` });

/** Whole days from today (local) to a YYYY-MM-DD date; negative when it has passed. */
export function daysUntil(dueOn: string, from = new Date()): number {
  const [y, m, d] = dueOn.split("-").map(Number);
  const due = Date.UTC(y!, (m ?? 1) - 1, d ?? 1);
  const today = Date.UTC(from.getFullYear(), from.getMonth(), from.getDate());
  return Math.round((due - today) / 86_400_000);
}

export function fmtDue(dueOn: string): { text: string; late: boolean } {
  const n = daysUntil(dueOn);
  if (n === 0) return { text: "due today", late: false };
  if (n === 1) return { text: "due tomorrow", late: false };
  if (n > 1) return { text: `due in ${n} days`, late: false };
  return { text: `${plural(-n, "day")} late`, late: true };
}

export function fmtDate(dueOn: string): string {
  const [y, m, d] = dueOn.split("-").map(Number);
  return new Date(y!, (m ?? 1) - 1, d ?? 1).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** Small chip naming the menu a task sits on. */
export function MenuChip({ name, className }: { name: string; className?: string }) {
  return (
    <span
      title={`On the ${name} menu`}
      className={cn(
        "inline-flex max-w-40 items-center gap-1 rounded-full border border-corn/50 bg-corn/15 px-2 py-px text-[11px] font-medium text-corn-text",
        className,
      )}
    >
      <Soup className="size-3 shrink-0" aria-hidden="true" />
      <span className="truncate">{name}</span>
    </span>
  );
}

export function MenuBar({ menu, className }: { menu: MenuView; className?: string }) {
  const full = menu.total > 0 && menu.done === menu.total;
  return (
    <progress
      aria-label={`${menu.name} progress`}
      value={menu.done}
      max={Math.max(1, menu.total)}
      className={cn(
        "block h-2 w-full appearance-none overflow-hidden rounded-full bg-muted",
        "[&::-webkit-progress-bar]:bg-muted [&::-webkit-progress-value]:rounded-full [&::-webkit-progress-value]:transition-[width] [&::-webkit-progress-value]:duration-500",
        "[&::-moz-progress-bar]:rounded-full",
        full
          ? "[&::-webkit-progress-value]:bg-success [&::-moz-progress-bar]:bg-success"
          : "[&::-webkit-progress-value]:bg-primary [&::-moz-progress-bar]:bg-primary",
        className,
      )}
    />
  );
}

function Counts({ menu }: { menu: MenuView }) {
  const b = (n: number) => <b className="font-semibold text-foreground">{n}</b>;
  return (
    <span>
      {b(menu.claimed)} on the fire · {b(menu.open)} to scoop · {b(menu.blocked)} waiting · {b(menu.done)} served
    </span>
  );
}

function DueText({ menu }: { menu: MenuView }) {
  if (!menu.due_on || menu.status === "served") return null;
  const d = fmtDue(menu.due_on);
  return (
    <span className={cn(d.late && "font-medium text-primary")} title={fmtDate(menu.due_on)}>
      {d.text}
    </span>
  );
}

/** Progress strip under the pot header while one menu is selected. */
export function MenuStrip({ menu }: { menu: MenuView }) {
  const served = menu.status === "served";
  return (
    <section aria-label={`${menu.name} menu`} className="mt-5 rounded-[14px] border border-border bg-card px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 className="flex items-center gap-2 text-[17px]">
          <Soup className="size-4 text-corn-text" aria-hidden="true" /> {menu.name}
          {served && <span className="rounded-full bg-success/15 px-2 py-px font-sans text-[11px] font-medium text-success">served</span>}
        </h2>
        <p className="text-[12.5px] text-muted-foreground">
          <DueText menu={menu} />
        </p>
      </div>
      <div className="mt-2.5 flex items-center gap-3">
        <MenuBar menu={menu} className="flex-1" />
        <span className="font-mono text-[12px] tabular-nums text-muted-foreground">
          {menu.done}/{menu.total}
        </span>
      </div>
      <p className="mt-2 text-[12.5px] text-muted-foreground">
        <Counts menu={menu} />
      </p>
    </section>
  );
}

// ---------------------------------------------------------------- panel

const NO_FILTERS = {};

/** A list of menu cards that scrolls with the page; it pages as its end comes into view when it is the page's last list, else on a "Show more" button. */
export function MenuCards({ label, source, render, more = "scroll" }: { label: string; source: ListSource<MenuView>; render: (m: MenuView) => React.ReactNode; more?: MoreMode }) {
  return (
    <VirtualList<MenuView>
      items={source.items}
      itemKey={(m) => m.key}
      renderItem={render}
      scroll="window"
      onEndReached={loadMore(source, more)}
      footer={listFooter(source, more)}
      aria-label={label}
      itemClassName="pb-2.5"
      defaultItemHeight={120}
    />
  );
}

/** The Menus tab: the open menus (paged, searchable) and, folded away, the served ones. */
export function MenusView({ potKey, onNew, onEdit }: { potKey: string; onNew: () => void; onEdit: (m: MenuView) => void }) {
  const call = useProcCall();
  const f = useUrlFilters(NO_FILTERS);
  const open = useMenusSource(potKey, { status: "open", q: f.q });
  const served = useMenusSource(potKey, { status: "served", q: f.q });
  const [serving, setServing] = useState<MenuView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [showServed, setShowServed] = useState(false);
  const searching = !!f.q.trim();

  async function reopen(m: MenuView) {
    setBusy(m.key);
    try {
      await call("menuUpdate", { project: potKey, key: m.key, status: "open" });
      toast.success(`${m.name} is back on the stove`);
    } catch (e) {
      toast.error("Could not reopen the menu", { description: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  }

  const loading = (open.isLoading && open.items.length === 0) || (served.isLoading && served.items.length === 0);
  const failed = (open.error && open.items.length === 0 ? open.error : null) ?? (served.error && served.items.length === 0 ? served.error : null);
  const none = !loading && !failed && open.items.length === 0 && served.items.length === 0 && !searching && !open.pending && !served.pending;
  const count = (s: ListSource<MenuView>) => `${s.items.length}${s.hasNextPage ? "+" : ""}`;

  return (
    <div className="mx-auto max-w-3xl">
      {loading ? (
        <RowSkeleton rows={3} />
      ) : failed ? (
        <EmptyState band={false} title="The menus would not load">
          {errorMessage(failed)}
        </EmptyState>
      ) : none ? (
        <EmptyState
          title="No menus yet"
          action={
            <Button onClick={onNew}>
              <Plus /> New menu
            </Button>
          }
        >
          A menu is a milestone, like MVP or v1.2. Put tasks on it, watch the bar fill, and serve it when it ships.
        </EmptyState>
      ) : (
        <>
          <FilterBar q={f.q} onQ={f.setQ} facets={[]} values={{}} onChange={() => {}} onClear={f.clear} searchPlaceholder="Search menus" />
          <div className="mt-4 flex items-center justify-between gap-3">
            <p className="label-caps">{open.items.length === 0 ? (searching ? "No open menu matches" : "Nothing is cooking right now") : `Cooking · ${count(open)}`}</p>
            <Button variant="outline" size="sm" onClick={onNew}>
              <Plus /> New menu
            </Button>
          </div>
          <div className="mt-3">
            <MenuCards label="Menus cooking" source={open} more={served.items.length > 0 ? "button" : "scroll"} render={(m) => <MenuCard potKey={potKey} menu={m} onEdit={() => onEdit(m)} onServe={() => setServing(m)} />} />
          </div>
          {served.items.length > 0 && (
            <section className="mt-7" aria-labelledby="served-menus">
              <h2 id="served-menus" className="m-0 text-base">
                <button
                  type="button"
                  aria-expanded={showServed}
                  onClick={() => setShowServed((o) => !o)}
                  className="flex w-full items-center gap-2 rounded-md py-1 text-left"
                >
                  <ChevronRight className={cn("size-4 text-muted-foreground transition-transform", showServed && "rotate-90")} aria-hidden="true" />
                  <span className="font-display text-[16px]">Served</span>
                  <span className="font-mono text-[11.5px] text-muted-foreground">{count(served)}</span>
                </button>
              </h2>
              {showServed && (
                <div className="mt-1.5">
                  <MenuCards
                    label="Menus served"
                    source={served}
                    render={(m) => <MenuCard potKey={potKey} menu={m} onEdit={() => onEdit(m)} onReopen={() => void reopen(m)} reopening={busy === m.key} />}
                  />
                </div>
              )}
            </section>
          )}
        </>
      )}
      <ServeMenuDialog potKey={potKey} menu={serving} onClose={() => setServing(null)} />
    </div>
  );
}

export function MenuCard({
  potKey,
  menu,
  onEdit,
  onServe,
  onReopen,
  reopening,
}: {
  potKey: string;
  menu: MenuView;
  onEdit: () => void;
  onServe?: () => void;
  onReopen?: () => void;
  reopening?: boolean;
}) {
  const served = menu.status === "served";
  const finished = menu.total > 0 && menu.open + menu.claimed + menu.blocked === 0;
  return (
    <div className={cn("rounded-[14px] border border-border bg-card px-3.5 py-3.5 md:px-4", served && "bg-card/60")}>
      <div className="flex items-start justify-between gap-x-3">
        <div className="min-w-0">
          <h3 className="flex flex-wrap items-center gap-x-2 text-[17px]">
            <Link
              to={`/pots/${potKey}/board?menu=${encodeURIComponent(menu.key)}`}
              className="rounded underline-offset-4 hover:underline focus-visible:underline"
            >
              {menu.name}
            </Link>
            <span className="font-mono text-[11px] font-normal text-muted-foreground">
              {menu.key}
            </span>
          </h3>
          {menu.description && <p className="mt-0.5 max-w-xl text-[13px] text-muted-foreground">{menu.description}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {onServe && finished && (
            <Button size="sm" onClick={onServe}>
              <PartyPopper /> Serve
            </Button>
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${menu.name}`}>
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-44">
              <DropdownMenuItem onSelect={onEdit} className="min-h-10">
                <Pencil /> Edit
              </DropdownMenuItem>
              {onServe && !finished && (
                <DropdownMenuItem onSelect={onServe} className="min-h-10">
                  <PartyPopper /> Serve menu
                </DropdownMenuItem>
              )}
              {onReopen && (
                <DropdownMenuItem onSelect={onReopen} disabled={reopening} className="min-h-10">
                  {reopening ? <Loader2 className="animate-spin" /> : <RotateCcw />} Reopen
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-3">
        <MenuBar menu={menu} className="flex-1" />
        <span className="font-mono text-[12px] tabular-nums text-muted-foreground">
          {menu.done}/{menu.total}
        </span>
      </div>
      <p className="mt-2 flex flex-wrap gap-x-2 gap-y-0.5 text-[12.5px] text-muted-foreground">
        <Counts menu={menu} />
        {menu.due_on && !served && (
          <>
            <span aria-hidden="true">·</span>
            <DueText menu={menu} />
          </>
        )}
        {served && menu.served_at && (
          <>
            <span aria-hidden="true">·</span>
            <span>served {fmtDate(menu.served_at.slice(0, 10))}</span>
          </>
        )}
      </p>
    </div>
  );
}

// ---------------------------------------------------------------- dialogs

/** Create (menu = null) or edit a menu. */
export function MenuDialog({
  potKey,
  open,
  menu,
  onClose,
  onSaved,
}: {
  potKey: string;
  open: boolean;
  menu: MenuView | null;
  onClose: () => void;
  onSaved?: (key: string) => void;
}) {
  const call = useProcCall();
  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [description, setDescription] = useState("");
  const [dueOn, setDueOn] = useState("");
  const [position, setPosition] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setName(menu?.name ?? "");
      setKey(menu?.key ?? "");
      setDescription(menu?.description ?? "");
      setDueOn(menu?.due_on ?? "");
      setPosition(menu ? String(menu.position) : "");
    }
    // reset only when the dialog opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    try {
      const pos = position.trim() === "" ? null : Number(position);
      const m = menu
        ? await call("menuUpdate", {
            project: potKey,
            key: menu.key,
            name: name.trim(),
            description,
            dueOn, // empty string clears it
            position: pos,
          })
        : await call("menuCreate", {
            project: potKey,
            name: name.trim(),
            key: key.trim() || null,
            description: description.trim() || null,
            dueOn: dueOn || null,
            position: pos,
          });
      toast.success(menu ? `${m.name} updated` : `${m.name} is on the menu`);
      onSaved?.(m.key);
      onClose();
    } catch (err) {
      toast.error(menu ? "Could not save the menu" : "Could not add the menu", { description: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{menu ? `Edit ${menu.name}` : "New menu"}</DialogTitle>
          <DialogDescription>A menu is a milestone: a set of tasks cooked together and served when they ship.</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
            <div className="space-y-1.5">
              <Label htmlFor="mn-name">Name</Label>
              <Input id="mn-name" autoFocus required value={name} onChange={(e) => setName(e.target.value)} placeholder="MVP" />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="mn-key">Key (optional)</Label>
              <Input
                id="mn-key"
                value={key}
                disabled={!!menu}
                onChange={(e) => setKey(e.target.value.toLowerCase())}
                placeholder="derived"
                className="font-mono text-[12.5px]"
              />
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="mn-desc">Description</Label>
            <Textarea id="mn-desc" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What this menu delivers." />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="mn-due">Due date</Label>
              <Input id="mn-due" type="date" value={dueOn} onChange={(e) => setDueOn(e.target.value)} />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="mn-pos">Position</Label>
              <Input id="mn-pos" type="number" inputMode="numeric" value={position} onChange={(e) => setPosition(e.target.value)} placeholder="after the last" />
            </div>
          </div>
          <DialogFooter className="gap-2 pt-1">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !name.trim()}>
              {busy && <Loader2 className="animate-spin" />} {menu ? "Save menu" : "Add the menu"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function ServeMenuDialog({ potKey, menu, onClose }: { potKey: string; menu: MenuView | null; onClose: () => void }) {
  const call = useProcCall();
  const [busy, setBusy] = useState(false);
  const left = menu ? menu.open + menu.claimed + menu.blocked : 0;
  return (
    <AlertDialog open={!!menu} onOpenChange={(o) => !o && !busy && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Serve the {menu?.name} menu?</AlertDialogTitle>
          <AlertDialogDescription>
            {left === 0
              ? "Every portion is out of the kitchen. Ring the bell and send it to the table."
              : `${plural(left, "portion")} ${left === 1 ? "is" : "are"} still cooking or waiting. Serving marks the menu shipped; those tasks stay in the pot and can still be cooked.`}{" "}
            You can reopen it later if more turns up.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Not yet</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy}
            onClick={async (e) => {
              e.preventDefault();
              if (!menu) return;
              setBusy(true);
              try {
                await call("menuUpdate", { project: potKey, key: menu.key, status: "served" });
                toast.success(`${menu.name} is served`, { description: "Bon appétit. The whole table thanks the cooks." });
                onClose();
              } catch (err) {
                toast.error("Could not serve the menu", { description: errorMessage(err) });
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? <Loader2 className="animate-spin" /> : <PartyPopper />} Serve it
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
