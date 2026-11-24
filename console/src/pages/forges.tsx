import { useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { ExternalLink, GitBranch, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { PageShell } from "@/components/page";
import { EmptyState, RowSkeleton } from "@/components/bits";
import { ConfirmDialog } from "@/components/rename-delete";
import { CopyLine } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useV1, useV1Call } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { cn } from "@/lib/utils";
import { FormDialog } from "@/pages/admin";

interface Installation {
  id: number;
  account: string | null;
  account_type: string | null;
  repo_selection: string | null;
  suspended: boolean;
}

interface Delivery {
  event: string;
  action: string | null;
  status: "received" | "ok" | "ignored" | "error";
  detail: string | null;
  at: string;
}

interface Connection {
  id: string;
  provider: string;
  name: string;
  api_url: string;
  web_url: string;
  app_id: number | null;
  slug: string | null;
  html_url: string | null;
  owner: string | null;
  install_url: string | null;
  created_at: string;
  created_by: string | null;
  installations: Installation[];
  last_delivery: Delivery | null;
}

const STATUS: Record<Delivery["status"], string> = {
  ok: "border-success/40 bg-success/10 text-success",
  ignored: "border-border bg-muted/60 text-muted-foreground",
  received: "border-corn/50 bg-corn/15 text-corn-text",
  error: "border-destructive/40 bg-destructive/10 text-destructive",
};

/** Admin: the git hosts corntchap listens to (GitHub Apps), where they are installed and what they last sent. */
export function ForgesPage() {
  const forges = useV1<{ public_url: string; connections: Connection[] }>("/forges");
  const call = useV1Call();
  const now = useNow();
  const [params, setParams] = useSearchParams();
  const [connecting, setConnecting] = useState(false);
  const [org, setOrg] = useState("");
  const [host, setHost] = useState("");
  const [removing, setRemoving] = useState<Connection | null>(null);

  // GitHub sends the person back here after creating the app
  useEffect(() => {
    if (params.get("connected")) {
      toast.success("GitHub is connected", { description: "Install the app on your account or organization to start getting events." });
      setParams({}, { replace: true });
    }
  }, [params, setParams]);

  const rows = forges.data?.connections ?? [];
  const publicUrl = forges.data?.public_url ?? "";
  const reachable = publicUrl.startsWith("https://");

  function connect() {
    const q = new URLSearchParams();
    if (org.trim()) q.set("org", org.trim());
    if (host.trim()) q.set("host", host.trim());
    // a full page load: GitHub shows its own screen and sends the browser back
    window.location.assign(`/forges/github/connect${q.size ? `?${q}` : ""}`);
  }

  async function remove(c: Connection) {
    try {
      await call("DELETE", `/forges/${c.id}`);
      toast.success(`${c.name} removed`);
    } catch (e) {
      toast.error("Could not remove it", { description: errorMessage(e) });
    }
  }

  return (
    <PageShell
      eyebrow="Admin"
      title="Forges"
      sub="Pull requests, checks and reviews from your git host, shown on the tasks they belong to. Nothing is imported and a merge never changes a task's status."
      actions={
        <Button onClick={() => setConnecting(true)}>
          <Plus /> Connect GitHub
        </Button>
      }
    >
      {publicUrl && !reachable && (
        <p className="mb-4 rounded-lg border border-corn/50 bg-corn/15 p-3 text-[13px]">
          GitHub sends events to <span className="font-mono">{publicUrl}/webhooks/github</span>. That address is not https, so GitHub on the internet cannot reach it: set PUBLIC_URL to the server's public address.
        </p>
      )}
      {forges.isLoading ? (
        <RowSkeleton rows={2} />
      ) : rows.length === 0 ? (
        <EmptyState title="No git host is connected" action={<Button onClick={() => setConnecting(true)}>Connect GitHub</Button>}>
          Connecting creates a read-only GitHub App for you in one click. Then install it on the accounts whose repositories your pots use.
        </EmptyState>
      ) : (
        <ul className="grid gap-3 lg:grid-cols-2">
          {rows.map((c) => (
            <li key={c.id} className="overflow-hidden rounded-[14px] border border-border bg-card">
              <div className="flex items-start gap-3 p-4">
                <span className="grid size-11 shrink-0 place-items-center rounded-xl bg-muted">
                  <GitBranch className="size-5 text-muted-foreground" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <h2 className="truncate font-display text-[18px] leading-tight">{c.name}</h2>
                  <p className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
                    {c.provider} · {c.web_url.replace(/^https?:\/\//, "")}
                    {c.owner && <> · owned by {c.owner}</>}
                    {c.created_by && <> · connected by {c.created_by}</>}
                  </p>
                </div>
                <Button variant="ghost" size="icon" className="shrink-0 text-muted-foreground max-md:size-11" aria-label={`Remove ${c.name}`} onClick={() => setRemoving(c)}>
                  <Trash2 />
                </Button>
              </div>

              <div className="space-y-3 border-t border-border px-4 py-3">
                <div>
                  <p className="label-caps">Installed on</p>
                  {c.installations.length === 0 ? (
                    <p className="mt-1 text-[13px] text-muted-foreground">Nowhere yet. Install the app to start getting events.</p>
                  ) : (
                    <ul className="mt-1.5 flex flex-wrap gap-1.5">
                      {c.installations.map((i) => (
                        <li key={i.id} className={cn("inline-flex h-7 items-center gap-1.5 rounded-full border border-border bg-muted/60 px-2.5 text-[12.5px]", i.suspended && "opacity-60")}>
                          <span className="font-medium">{i.account ?? `installation ${i.id}`}</span>
                          {i.repo_selection && <span className="text-muted-foreground">{i.repo_selection === "all" ? "all repositories" : "selected repositories"}</span>}
                          {i.suspended && <span className="text-destructive">suspended</span>}
                        </li>
                      ))}
                    </ul>
                  )}
                  {c.install_url && (
                    <Button asChild variant="outline" size="sm" className="mt-2.5 max-md:h-11">
                      <a href={c.install_url} target="_blank" rel="noreferrer">
                        Install on an org <ExternalLink />
                      </a>
                    </Button>
                  )}
                </div>

                <div>
                  <p className="label-caps">Last delivery</p>
                  {c.last_delivery ? (
                    <p className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
                      <span className={cn("inline-flex h-6 items-center rounded-full border px-2 text-[11.5px] font-medium", STATUS[c.last_delivery.status])}>{c.last_delivery.status}</span>
                      <span className="font-mono text-[12px]">
                        {c.last_delivery.event}
                        {c.last_delivery.action && `.${c.last_delivery.action}`}
                      </span>
                      <span className="text-muted-foreground">{fmtAgo(c.last_delivery.at, now)}</span>
                      {c.last_delivery.detail && <span className="min-w-0 max-w-full truncate text-muted-foreground">{c.last_delivery.detail}</span>}
                    </p>
                  ) : (
                    <p className="mt-1 text-[13px] text-muted-foreground">Nothing received yet.</p>
                  )}
                </div>
              </div>
              <div className="border-t border-border bg-muted/30 p-2.5">
                <CopyLine text={`${publicUrl}/webhooks/${c.provider}`} />
              </div>
            </li>
          ))}
        </ul>
      )}

      <FormDialog
        open={connecting}
        onOpenChange={setConnecting}
        title="Connect GitHub"
        description="GitHub will ask you to create a read-only app for corntchap. You come back here when it is done."
        onSubmit={connect}
        busy={false}
        submitLabel="Continue to GitHub"
      >
        <div className="space-y-1.5">
          <Label htmlFor="forge-org">Organization (optional)</Label>
          <Input id="forge-org" autoCapitalize="none" spellCheck={false} value={org} onChange={(e) => setOrg(e.target.value)} placeholder="acme" className="h-11 font-mono" />
          <p className="text-[12.5px] text-muted-foreground">Leave empty to create the app under your own account.</p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="forge-host">GitHub Enterprise host (optional)</Label>
          <Input id="forge-host" autoCapitalize="none" spellCheck={false} value={host} onChange={(e) => setHost(e.target.value)} placeholder="github.example.com" className="h-11 font-mono" />
        </div>
      </FormDialog>
      <ConfirmDialog
        open={!!removing}
        onOpenChange={(o) => !o && setRemoving(null)}
        title={`Remove ${removing?.name ?? ""}?`}
        description="corntchap stops listening to this app. Pull requests it already told about stay on their tasks. The app itself stays on GitHub: delete it there too if you do not need it."
        action="Remove"
        destructive
        onConfirm={async () => {
          if (removing) await remove(removing);
        }}
      />
    </PageShell>
  );
}
