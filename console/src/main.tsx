import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Outlet, Route, Routes } from "react-router";
import { Refine, type NotificationProvider } from "@refinedev/core";
import routerProvider from "@refinedev/react-router";
import { toast } from "sonner";
import "./index.css";
import { dataProvider } from "@/lib/api";
import { authProvider, SessionProvider } from "@/lib/session";
import { ThemeProvider, useTheme } from "@/lib/theme";
import { UiProvider } from "@/lib/ui";
import { LiveProvider } from "@/lib/live";
import { AppShell } from "@/components/shell";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { KitchenPage, PotRedirect } from "@/pages/kitchen";
import { CooksPage } from "@/pages/cooks";
import { ConnectPage } from "@/pages/connect";
import { AdminOnly, FiresPage, PotsAdminPage, TeamsAdminPage, UsersAdminPage } from "@/pages/admin";
import { EmptyState } from "@/components/bits";
import { Button } from "@/components/ui/button";
import { Link } from "react-router";

// Refine's notification hook, rendered as sonner toasts (errors from data hooks surface here too).
const notificationProvider: NotificationProvider = {
  open: ({ message, description, type, key }) => {
    const opts = { ...(key ? { id: key } : {}), ...(description ? { description } : {}) };
    if (type === "error") toast.error(message, opts);
    else if (type === "success") toast.success(message, opts);
    else toast(message, opts);
  },
  close: (key) => {
    toast.dismiss(key);
  },
};

function NotFound() {
  return (
    <div className="mx-auto max-w-2xl px-5 py-16">
      <EmptyState title="Nothing cooking at this address" action={<Button asChild variant="outline"><Link to="/">Back to the kitchen</Link></Button>}>
        The page you asked for is not in this kitchen.
      </EmptyState>
    </div>
  );
}

function ThemedToaster() {
  const { resolved } = useTheme();
  return <Toaster theme={resolved} position="bottom-right" closeButton />;
}

function Providers() {
  return (
    <Refine
      dataProvider={dataProvider}
      routerProvider={routerProvider}
      authProvider={authProvider}
      notificationProvider={notificationProvider}
      resources={[
        { name: "projects", list: "/admin/pots" },
        { name: "principals", list: "/cooks" },
        { name: "teams", list: "/teams" },
      ]}
      options={{ disableTelemetry: true, syncWithLocation: false, warnWhenUnsavedChanges: false, reactQuery: { clientConfig: { defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } } } }}
    >
      <SessionProvider>
       <LiveProvider>
        <UiProvider>
          <Routes>
            <Route element={<AppShell />}>
              <Route index element={<PotRedirect />} />
              <Route path="pots/:key" element={<PotRedirect />} />
              <Route path="pots/:key/v/:viewKey" element={<KitchenPage />} />
              <Route path="pots/:key/:view" element={<KitchenPage />} />
              <Route path="cooks" element={<CooksPage />} />
              <Route path="connect/:code" element={<ConnectPage />} />
              <Route path="fires" element={<FiresPage />} />
              <Route path="teams" element={<TeamsAdminPage />} />
              <Route path="admin/teams" element={<Navigate to="/teams" replace />} />
              <Route element={<AdminOnly><Outlet /></AdminOnly>}>
                <Route path="admin/pots" element={<PotsAdminPage />} />
                <Route path="admin/users" element={<UsersAdminPage />} />
              </Route>
              <Route path="*" element={<NotFound />} />
            </Route>
          </Routes>
        </UiProvider>
       </LiveProvider>
      </SessionProvider>
      <ThemedToaster />
    </Refine>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider>
      <TooltipProvider delayDuration={200}>
        <BrowserRouter>
          <Providers />
        </BrowserRouter>
      </TooltipProvider>
    </ThemeProvider>
  </StrictMode>,
);
