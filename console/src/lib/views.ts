/** The built-in tabs (a pot's task view is the Board; saved SQL views live on the server). */
export type BaseView = "board" | "activity" | "pantry" | "menus" | "questions" | "pass" | "files";

const BASES: BaseView[] = ["board", "activity", "pantry", "menus", "questions", "pass", "files"];

export function isBaseView(v: string | undefined): v is BaseView {
  return !!v && (BASES as string[]).includes(v);
}

export interface Filters {
  repo?: string;
  status?: string;
  label?: string;
  holder?: string;
  /** a menu key, or "-" for tasks on no menu */
  menu?: string;
  q?: string;
}

const lastKey = (pot: string) => `corntchap.lastview.${pot}`;

/** The path segment under `/pots/:key/` to open: a built-in tab (`board`) or a saved view (`v/<key>`). */
export function lastView(pot: string): string {
  try {
    const v = localStorage.getItem(lastKey(pot));
    if (v && isBaseView(v)) return v;
    if (v && /^v\/[^/]+$/.test(v)) return v;
  } catch {
    /* ignore */
  }
  return "board";
}

export function rememberView(pot: string, v: BaseView | `v/${string}`) {
  try {
    localStorage.setItem(lastKey(pot), v);
  } catch {
    /* ignore */
  }
}

/** Forget a saved view that no longer exists. */
export function forgetView(pot: string, viewKey: string) {
  try {
    if (localStorage.getItem(lastKey(pot)) === `v/${viewKey}`) localStorage.removeItem(lastKey(pot));
  } catch {
    /* ignore */
  }
}

const LAST_POT = "corntchap.lastpot";

/** The pot opened last: phones keep their bottom tabs on it while on other pages. */
export function lastPot(): string | null {
  try {
    return localStorage.getItem(LAST_POT);
  } catch {
    return null;
  }
}

export function rememberPot(pot: string) {
  try {
    localStorage.setItem(LAST_POT, pot.toUpperCase());
  } catch {
    /* ignore */
  }
}
