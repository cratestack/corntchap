import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { Link, useParams } from "react-router";
import type { CloudSetup } from "@corntchap/api";
import { CheckCircle2, Copy, Loader2, Send } from "lucide-react";
import { toast } from "sonner";
import { PageShell } from "@/components/page";
import { EmptyState, RowSkeleton } from "@/components/bits";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";

const KIND_TITLE: Record<string, string> = {
  "claude-routine": "a Claude Code routine",
  "github-comment": "GitHub comments",
  webhook: "a webhook",
};

const randomSecret = () => {
  const b = new Uint8Array(24);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
};

const STATUS_LABEL: Record<string, string> = {
  waiting: "Waiting for values",
  connected: "Connected",
  expired: "Expired",
  removed: "Removed",
};

export function ConnectPage() {
  const { code = "" } = useParams();
  const call = useProcCall();
  const callRef = useRef(call);
  callRef.current = call;
  const now = useNow();
  const [setup, setSetup] = useState<CloudSetup | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  const prefill = useCallback((s: CloudSetup) => {
    if (s.kind === "webhook" && s.status === "waiting" && s.fields.some((f) => f.name === "secret")) {
      setValues((v) => (v.secret ? v : { ...v, secret: randomSecret() }));
    }
  }, []);

  useEffect(() => {
    let live = true;
    setSetup(null);
    setError(null);
    callRef.current("cloudConnect", { code })
      .then((s) => {
        if (!live) return;
        setSetup(s);
        prefill(s);
      })
      .catch((e: unknown) => {
        if (live) setError(errorMessage(e));
      });
    return () => {
      live = false;
    };
  }, [code, prefill]);

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!setup) return;
    setSaving(true);
    try {
      const v = (n: string) => (values[n] ?? "").trim() || null;
      const issueRaw = v("issue");
      const next = await call("cloudConnectSave", {
        code,
        url: v("url"),
        token: v("token"),
        secret: v("secret"),
        repo: v("repo"),
        issue: issueRaw === null ? null : Number(issueRaw),
        mention: v("mention"),
      });
      setValues({});
      setSetup(next);
      toast.success("Connected");
    } catch (err) {
      toast.error("Could not connect", { description: errorMessage(err) });
    } finally {
      setSaving(false);
    }
  }

  async function sendTest() {
    setTesting(true);
    try {
      setSetup(await call("cloudConnect", { code, test: true }));
    } catch (err) {
      toast.error("Test wake failed", { description: errorMessage(err) });
    } finally {
      setTesting(false);
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success("Copied");
    } catch {
      toast.error("Could not copy");
    }
  }

  if (error || !setup) {
    return (
      <PageShell title="Connect a cook" eyebrow="Cloud setup">
        {error ? (
          <EmptyState title="This setup is not available" action={<Button asChild variant="outline"><Link to="/cooks">Back to the cooks</Link></Button>}>
            {error}
          </EmptyState>
        ) : (
          <RowSkeleton rows={3} />
        )}
      </PageShell>
    );
  }

  const s = setup;
  const waiting = s.status === "waiting";
  const connected = s.status === "connected";
  const canSave = s.fields.every((f) => !f.required || (values[f.name] ?? "").trim() !== "");

  return (
    <PageShell
      title={`Connect ${s.cook} to ${KIND_TITLE[s.kind] ?? s.kind}`}
      eyebrow="Cloud setup"
      sub={
        <>
          {STATUS_LABEL[s.status] ?? s.status}
          {waiting && <> · expires {new Date(s.expires_at).toLocaleString()}</>}
          {s.projects.length > 0 && <> · pots: {s.projects.join(", ")}</>}
        </>
      }
    >
      <div className="max-w-2xl space-y-5">
        {(s.status === "expired" || s.status === "removed") && (
          <EmptyState title={s.status === "expired" ? "This setup has expired" : "This setup was removed"} band={false}>
            Ask your agent for a new setup: connect_cloud.
          </EmptyState>
        )}

        {waiting && s.steps.length > 0 && (
          <section className="rounded-lg border border-border bg-card p-4" aria-label="Steps">
            <div className="label-caps">Steps</div>
            <ol className="mt-2 list-decimal space-y-1.5 pl-5 text-[12.5px]">
              {s.steps.map((st, i) => (
                <li key={i}>{st}</li>
              ))}
            </ol>
          </section>
        )}

        {s.kind === "claude-routine" && s.routine_prompt && (waiting || connected) && (
          <section className="rounded-lg border border-border bg-card p-4" aria-label="Routine prompt">
            <div className="flex items-center justify-between gap-2">
              <div className="label-caps">Routine prompt</div>
              <Button type="button" variant="outline" size="sm" onClick={() => void copy(s.routine_prompt ?? "")}>
                <Copy className="size-3.5" /> Copy
              </Button>
            </div>
            <pre
              tabIndex={0}
              aria-label="Routine prompt text"
              className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-background p-3 font-mono text-[12px]"
            >
              {s.routine_prompt}
            </pre>
          </section>
        )}

        {waiting && (
          <form onSubmit={(e) => void save(e)} className="space-y-4 rounded-lg border border-border bg-card p-4">
            <div className="label-caps">Values</div>
            {s.fields.map((f) => {
              const id = `connect-${f.name}`;
              const isWebhookSecret = s.kind === "webhook" && f.name === "secret";
              return (
                <div key={f.name} className="space-y-1.5">
                  <Label htmlFor={id} className="text-[12.5px]">
                    {f.label}
                    {f.required && <span aria-hidden="true" className="ml-0.5 text-destructive">*</span>}
                    {f.required && <span className="sr-only"> (required)</span>}
                  </Label>
                  <Input
                    id={id}
                    name={f.name}
                    type={isWebhookSecret ? "text" : f.secret ? "password" : f.name === "issue" ? "number" : "text"}
                    min={f.name === "issue" ? 1 : undefined}
                    autoComplete="off"
                    spellCheck={false}
                    required={f.required}
                    placeholder={f.placeholder}
                    value={values[f.name] ?? ""}
                    onChange={(e) => setValues((v) => ({ ...v, [f.name]: e.target.value }))}
                    aria-describedby={f.help || isWebhookSecret ? `${id}-help` : undefined}
                    className={isWebhookSecret ? "font-mono text-[12.5px]" : "text-[12.5px]"}
                  />
                  {(f.help || isWebhookSecret) && (
                    <p id={`${id}-help`} className="text-[12px] text-muted-foreground">
                      {f.help}
                      {isWebhookSecret && <> Copy it to your endpoint before you connect.</>}
                    </p>
                  )}
                </div>
              );
            })}
            <Button type="submit" disabled={saving || !canSave}>
              {saving && <Loader2 className="size-4 animate-spin" />} Connect
            </Button>
          </form>
        )}

        {connected && (
          <section className="space-y-3 rounded-lg border border-border bg-card p-4" aria-label="Connected">
            <div className="flex items-center gap-2 text-[13.5px]">
              <CheckCircle2 className="size-4 text-emerald-600" aria-hidden="true" />
              <span>corntchap wakes {s.cook} this way from now on.</span>
            </div>
            {s.target && (
              <p className="text-[12.5px] text-muted-foreground">
                Target: <code className="font-mono text-foreground">{s.target.id}</code>
                {s.target.type && <> ({s.target.type})</>}
              </p>
            )}
            <div>
              <Button type="button" variant="outline" size="sm" disabled={testing} onClick={() => void sendTest()}>
                {testing ? <Loader2 className="size-3.5 animate-spin" /> : <Send className="size-3.5" />} Send a test wake
              </Button>
            </div>
            {s.test && (
              <div className="rounded-md border border-border bg-background p-3 text-[12.5px]" role="status">
                <div className="label-caps">Last test, {fmtAgo(s.test.at, now)}</div>
                {s.test.result.ok ? (
                  <p className="mt-1">
                    Delivered{s.test.result.status != null && <> (HTTP {s.test.result.status})</>}.
                    {s.test.result.session_url && (
                      <>
                        {" "}
                        <a href={s.test.result.session_url} target="_blank" rel="noreferrer" className="underline underline-offset-2">
                          Open the session
                        </a>
                      </>
                    )}
                  </p>
                ) : (
                  <p className="mt-1 text-destructive">
                    Failed{s.test.result.status != null && <> (HTTP {s.test.result.status})</>}
                    {s.test.result.error && <>: {s.test.result.error}</>}
                  </p>
                )}
              </div>
            )}
          </section>
        )}
      </div>
    </PageShell>
  );
}
