import { createContext, useContext, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import type { AuthProvider } from "@refinedev/core";
import { CONSOLE_HEADER, errorStatus, redirectToLogin } from "./api";
import { LogoMark } from "@/components/brand";

export interface SessionUser {
  id: string;
  name: string;
  kind: string;
  is_admin: boolean;
  /** CSRF token for the server-rendered POST /logout. */
  csrf: string | null;
}

const Ctx = createContext<SessionUser | null>(null);

async function fetchSession(): Promise<SessionUser> {
  const res = await fetch("/auth/session", { credentials: "same-origin", headers: CONSOLE_HEADER });
  if (res.status === 401) {
    redirectToLogin();
    return new Promise<SessionUser>(() => {});
  }
  if (!res.ok) throw new Error(`Could not load your session (${res.status})`);
  return (await res.json()) as SessionUser;
}

export function SessionProvider({ children }: { children: ReactNode }) {
  const q = useQuery({ queryKey: ["session"], queryFn: fetchSession, staleTime: Infinity, retry: 1 });
  if (q.isError) {
    return (
      <div className="grid min-h-dvh place-items-center bg-background p-6 text-center">
        <div className="max-w-sm">
          <LogoMark className="mx-auto size-10" />
          <h1 className="mt-4 text-2xl">The pot is out of reach</h1>
          <p className="mt-2 text-muted-foreground">
            {(q.error as Error).message}. Check that the server is running, then try again.
          </p>
          <button
            className="mt-5 rounded-lg border border-input bg-card px-4 py-2 font-medium"
            onClick={() => void q.refetch()}
          >
            Try again
          </button>
        </div>
      </div>
    );
  }
  if (!q.data) {
    return (
      <div className="grid min-h-dvh place-items-center bg-background" aria-busy="true" aria-label="Loading">
        <LogoMark className="size-12 animate-pulse" />
      </div>
    );
  }
  return <Ctx.Provider value={q.data}>{children}</Ctx.Provider>;
}

export function useSession(): SessionUser {
  const s = useContext(Ctx);
  if (!s) throw new Error("useSession outside SessionProvider");
  return s;
}

/** Refine auth hooks: a 401 anywhere sends the browser to the OIDC sign-in and back. */
export const authProvider: AuthProvider = {
  login: async () => ({ success: true }),
  logout: async () => ({ success: true }),
  check: async () => ({ authenticated: true }),
  onError: async (error) => {
    if (errorStatus(error) === 401) {
      redirectToLogin();
      return { logout: false };
    }
    return {};
  },
};

export function logout(csrf: string | null) {
  const f = document.createElement("form");
  f.method = "post";
  f.action = "/logout";
  const i = document.createElement("input");
  i.type = "hidden";
  i.name = "csrf";
  i.value = csrf ?? "";
  f.appendChild(i);
  document.body.appendChild(f);
  f.submit();
}
