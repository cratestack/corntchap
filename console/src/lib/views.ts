// "+ View" stub: a saved view is a base view (list/board/activity/pantry) plus its filter chips, kept per pot in localStorage.
export type BaseView = "list" | "board" | "activity" | "pantry" | "menus" | "questions" | "files";

export interface Filters {
  status?: string;
  label?: string;
  holder?: string;
  /** a menu key, or "-" for tasks on no menu */
  menu?: string;
  q?: string;
}

export interface SavedView {
  id: string;
  name: string;
  base: BaseView;
  filters: Filters;
}

const key = (pot: string) => `corntchap.views.${pot}`;

export function loadViews(pot: string): SavedView[] {
  try {
    const raw = localStorage.getItem(key(pot));
    if (!raw) return [];
    const parsed = JSON.parse(raw) as SavedView[];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function saveViews(pot: string, views: SavedView[]) {
  try {
    localStorage.setItem(key(pot), JSON.stringify(views));
  } catch {
    /* storage unavailable: the stub just does not persist */
  }
}

export function lastView(pot: string): BaseView {
  try {
    const v = localStorage.getItem(`corntchap.lastview.${pot}`);
    if (v === "list" || v === "board" || v === "activity" || v === "pantry" || v === "menus" || v === "questions" || v === "files") return v;
  } catch {
    /* ignore */
  }
  return "list";
}

export function rememberView(pot: string, v: BaseView) {
  try {
    localStorage.setItem(`corntchap.lastview.${pot}`, v);
  } catch {
    /* ignore */
  }
}
