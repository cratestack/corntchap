export const DEFAULT_URL = "https://corntchap.camer.digital";
const RECENT_KEY = "corntchap.recentServers";
const MAX_RECENT = 4;

/** Turns what the person typed into a bare origin, or null. A bare host gets https://. */
export function normalise(raw: string | null | undefined): string | null {
  let v = (raw ?? "").trim();
  if (!v) return null;
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(v)) v = "https://" + v;
  try {
    const u = new URL(v);
    if (u.protocol !== "https:" && u.protocol !== "http:") return null;
    if (!u.hostname) return null;
    return u.origin;
  } catch {
    return null;
  }
}

export function hostOf(origin: string): string {
  try {
    return new URL(origin).host;
  } catch {
    return origin;
  }
}

export function loadRecent(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string").slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}

export function rememberServer(origin: string): void {
  try {
    const next = [origin, ...loadRecent().filter((x) => x !== origin)].slice(0, MAX_RECENT);
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* storage unavailable: recents are a nicety */
  }
}

export function forgetServer(origin: string): string[] {
  const next = loadRecent().filter((x) => x !== origin);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* ignore */
  }
  return next;
}
