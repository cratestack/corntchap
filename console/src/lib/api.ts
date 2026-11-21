import { CorntchapApiClient, cratestackRefineResources } from "@corntchap/api";
import { createCratestackDataProvider } from "@cratestack/refine";

/**
 * Browser auth: the session cookie only counts when a request carries this custom header. A cross-site page cannot
 * send it without a CORS preflight, which the server never grants (see crates/server/src/auth.rs).
 */
export const CONSOLE_HEADER = { "x-corntchap-console": "1" } as const;

export class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

export function loginUrl(next = window.location.pathname + window.location.search): string {
  return `/auth/oidc/login?next=${encodeURIComponent(next)}`;
}

let redirecting = false;
export function redirectToLogin() {
  if (redirecting) return;
  redirecting = true;
  window.location.assign(loginUrl());
}

/** Plain JSON call to a hand-written /api/v1 endpoint (used only where /api/v2 has no equivalent). */
export async function v1<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    credentials: "same-origin",
    headers: { ...CONSOLE_HEADER, ...(body !== undefined ? { "content-type": "application/json" } : {}) },
    body: body !== undefined ? JSON.stringify(body) : null,
  });
  const text = await res.text();
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    /* not json */
  }
  if (!res.ok) {
    if (res.status === 401) redirectToLogin();
    const j = (json ?? {}) as { error?: string; message?: string };
    throw new ApiError(res.status, j.error ?? "error", j.message ?? `Request failed (${res.status})`);
  }
  return json as T;
}

export const client = new CorntchapApiClient(window.location.origin, {
  basePath: "/api/v2",
  headers: CONSOLE_HEADER,
});

// `cratestackRefineResources` covers the models; procedures have no manifest, so name them here.
const procedureNames = Object.getOwnPropertyNames(Object.getPrototypeOf(client.procedures)).filter(
  (n) => n !== "constructor",
);
const procedures = Object.fromEntries(
  procedureNames.map((n) => [
    n,
    (args: unknown) => (client.procedures as unknown as Record<string, (a: unknown) => Promise<unknown>>)[n]!(args),
  ]),
);

export const dataProvider = createCratestackDataProvider(cratestackRefineResources(client), { procedures });

/** Human-readable message from anything thrown by the generated client, the data provider or `v1`. */
export function errorMessage(e: unknown): string {
  if (!e || typeof e !== "object") return String(e ?? "Something went wrong");
  const x = e as { payload?: { message?: string }; body?: { message?: string }; message?: string };
  return x.body?.message ?? x.payload?.message ?? x.message ?? "Something went wrong";
}

export function errorStatus(e: unknown): number | undefined {
  const x = e as { status?: number; statusCode?: number } | null;
  return x?.status ?? x?.statusCode;
}
