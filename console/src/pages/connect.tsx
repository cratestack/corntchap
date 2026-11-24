import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, useParams } from "react-router";
import type { CloudSetup } from "@corntchap/api";
import { ArrowLeft, Check, CheckCircle2, Loader2, Send, XCircle } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { PageShell } from "@/components/page";
import { EmptyState, RowSkeleton } from "@/components/bits";
import { CodeBlock, CopyButton } from "@/components/secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";

const KIND_TITLE: Record<string, string> = {
  "claude-routine": "a Claude Code routine",
  "github-comment": "GitHub comments",
  webhook: "a webhook",
  coder: "a Coder agent",
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

/** The three stages as a row of numbered dots: done ones get a tick, the current one is terracotta. */
function Progress({ at }: { at: 1 | 2 | 3 }) {
  const items = ["Prepare", "Fill in", "Test"];
  return (
    <ol className="flex items-center gap-2" aria-label="Setup progress">
      {items.map((label, i) => {
        const n = i + 1;
        const done = n < at;
        const now = n === at;
        return (
          <li key={label} className="flex min-w-0 flex-1 items-center gap-2" aria-current={now ? "step" : undefined}>
            <span
              className={cn(
                "grid size-6 shrink-0 place-items-center rounded-full border font-mono text-[11px] font-bold",
                done ? "border-success bg-success text-white" : now ? "border-primary bg-primary text-primary-foreground" : "border-border text-muted-foreground",
              )}
            >
              {done ? <Check className="size-3.5" aria-hidden="true" /> : n}
            </span>
            <span className={cn("truncate text-[12.5px]", now ? "font-semibold" : "text-muted-foreground")}>{label}</span>
            {n < 3 && <span aria-hidden="true" className={cn("h-px flex-1", done ? "bg-success" : "bg-border")} />}
          </li>
        );
      })}
    </ol>
  );
}

function Card({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <section aria-label={title} className={cn("min-w-0 rounded-[14px] border border-border bg-card p-4", className)}>
      <h2 className="font-display text-[19px] leading-tight">{title}</h2>
      <div className="mt-3">{children}</div>
    </section>
  );
}

export function ConnectPage() {
  const { code = "" } = useParams();
  const call = useProcCall();
  const callRef = useRef(call);
  callRef.current = call;
  const now = useNow();
  const [setup, setSetup] = useState<CloudSetup | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [takesWork, setTakesWork] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  // phones show one step at a time; wider screens show the steps together
  const [step, setStep] = useState<1 | 2>(1);

  const prefill = useCallback((s: CloudSetup) => {
    if (s.kind === "webhook" && s.status === "waiting" && s.fields.some((f) => f.name === "secret")) {
      setValues((v) => (v.secret ? v : { ...v, secret: randomSecret() }));
    }
  }, []);

  useEffect(() => {
    let live = true;
    setSetup(null);
    setError(null);
    setStep(1);
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
        template: v("template"),
        preset: v("preset"),
        owner: v("owner"),
        organization: v("organization"),
        workspace: v("workspace"),
        takesWork,
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
  const at: 1 | 2 | 3 = connected ? 3 : step;
  const hasPrepare = waiting && (s.steps.length > 0 || (s.kind === "claude-routine" && !!s.routine_prompt));

  return (
    <PageShell
      title={`Connect ${s.cook}`}
      eyebrow={`Cloud setup · ${KIND_TITLE[s.kind] ?? s.kind}`}
      sub={
        <>
          <span className={cn("font-medium", connected ? "text-success" : "text-foreground")}>{STATUS_LABEL[s.status] ?? s.status}</span>
          {waiting && <> · expires {new Date(s.expires_at).toLocaleString()}</>}
          {s.projects.length > 0 && (
            <>
              {" "}
              · pots <span className="font-mono">{s.projects.join(", ")}</span>
            </>
          )}
        </>
      }
    >
      <div className="max-w-2xl space-y-4">
        {(waiting || connected) && <Progress at={at} />}

        {(s.status === "expired" || s.status === "removed") && (
          <EmptyState title={s.status === "expired" ? "This setup has expired" : "This setup was removed"} band={false}>
            Ask your agent for a new setup: connect_cloud.
          </EmptyState>
        )}

        {/* step 1: what to do on the other side */}
        {hasPrepare && (
          <div className={cn("space-y-4", step !== 1 && "max-md:hidden")}>
            {s.steps.length > 0 && (
              <Card title="Do this first">
                <ol className="space-y-3">
                  {s.steps.map((st, i) => (
                    <li key={i} className="flex gap-3 text-[14px] leading-snug">
                      <span className="mt-px grid size-6 shrink-0 place-items-center rounded-full bg-muted font-mono text-[11.5px] font-bold">{i + 1}</span>
                      <span className="min-w-0 flex-1 break-words">{st}</span>
                    </li>
                  ))}
                </ol>
              </Card>
            )}
            {s.kind === "claude-routine" && s.routine_prompt && (
              <Card title="Routine prompt">
                <p className="mb-2 text-[12.5px] text-muted-foreground">Paste this as the routine's prompt. It tells each cloud run which cook it is.</p>
                <CodeBlock label="Routine prompt text" wrap>
                  {s.routine_prompt}
                </CodeBlock>
                <CopyButton text={s.routine_prompt} label="Copy the prompt" block variant="default" className="mt-3" />
              </Card>
            )}
          </div>
        )}

        {/* step 2: the values */}
        {waiting && (
          <form id="connect-form" onSubmit={(e) => void save(e)} className={cn("space-y-4 rounded-[14px] border border-border bg-card p-4", hasPrepare && step !== 2 && "max-md:hidden")}>
            <h2 className="font-display text-[19px] leading-tight">Paste the values</h2>
            {s.fields.map((f) => {
              const id = `connect-${f.name}`;
              const isWebhookSecret = s.kind === "webhook" && f.name === "secret";
              return (
                <div key={f.name} className="space-y-1.5">
                  <Label htmlFor={id} className="text-[13px]">
                    {f.label}
                    {f.required && <span aria-hidden="true" className="ml-0.5 text-destructive">*</span>}
                    {f.required && <span className="sr-only"> (required)</span>}
                  </Label>
                  <Input
                    id={id}
                    name={f.name}
                    type={isWebhookSecret ? "text" : f.secret ? "password" : f.name === "issue" ? "number" : "text"}
                    inputMode={f.name === "issue" ? "numeric" : undefined}
                    min={f.name === "issue" ? 1 : undefined}
                    autoComplete="off"
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    required={f.required}
                    placeholder={f.placeholder}
                    value={values[f.name] ?? ""}
                    onChange={(e) => setValues((v) => ({ ...v, [f.name]: e.target.value }))}
                    aria-describedby={f.help || isWebhookSecret ? `${id}-help` : undefined}
                    className={cn("h-11", isWebhookSecret && "font-mono text-[12.5px]")}
                  />
                  {isWebhookSecret && <CopyButton text={values[f.name] ?? ""} label="Copy the secret" block className="mt-1" />}
                  {(f.help || isWebhookSecret) && (
                    <p id={`${id}-help`} className="text-[12px] text-muted-foreground">
                      {f.help}
                      {isWebhookSecret && <> Copy it to your endpoint before you connect.</>}
                    </p>
                  )}
                </div>
              );
            })}
            <label htmlFor="connect-takes-work" className="flex min-h-14 cursor-pointer items-start gap-3 rounded-xl border border-border bg-muted/40 p-3.5">
              <Checkbox id="connect-takes-work" checked={takesWork} onCheckedChange={(c) => setTakesWork(c === true)} className="mt-0.5 size-5" />
              <span className="space-y-0.5">
                <span className="block text-[14px] font-medium">Also start it for new work</span>
                <span className="block text-[12.5px] leading-snug text-muted-foreground">
                  Off: {s.cook} is on call. A direct message, an @mention or a comment on its task starts a run; broadcasts and answers to its questions do not (it reads answers on its next run). On: every ready task in its pots starts a run too.
                </span>
              </span>
            </label>
            <Button type="submit" disabled={saving || !canSave} className="h-11 max-md:hidden">
              {saving && <Loader2 className="size-4 animate-spin" />} Connect
            </Button>
          </form>
        )}

        {/* step 3: connected, then test */}
        {connected && (
          <Card title="Connected" className="space-y-3">
            <div className="flex items-center gap-2 text-[14px]">
              <CheckCircle2 className="size-5 shrink-0 text-success" aria-hidden="true" />
              <span>corntchap wakes {s.cook} this way from now on.</span>
            </div>
            {s.target && (
              <p className="text-[12.5px] text-muted-foreground">
                Target: <code className="font-mono text-foreground">{s.target.id}</code>
                {s.target.type && <> ({s.target.type})</>}
              </p>
            )}
            <Button type="button" variant="outline" className="h-11 w-full sm:w-auto" disabled={testing} onClick={() => void sendTest()}>
              {testing ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />} Send a test wake
            </Button>
            {s.test && (
              <div className="rounded-xl border border-border bg-background p-3.5 text-[13px]" role="status">
                <div className="label-caps">Last test, {fmtAgo(s.test.at, now)}</div>
                {s.test.result.ok ? (
                  <p className="mt-1.5 flex items-start gap-1.5">
                    <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
                    <span>
                      Delivered{s.test.result.status != null && <> (HTTP {s.test.result.status})</>}.
                      {s.test.result.session_url && (
                        <>
                          {" "}
                          <a href={s.test.result.session_url} target="_blank" rel="noreferrer" className="underline underline-offset-2">
                            Open the session
                          </a>
                        </>
                      )}
                    </span>
                  </p>
                ) : (
                  <p className="mt-1.5 flex items-start gap-1.5 text-destructive">
                    <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                    <span>
                      Failed{s.test.result.status != null && <> (HTTP {s.test.result.status})</>}
                      {s.test.result.error && <>: {s.test.result.error}</>}
                    </span>
                  </p>
                )}
              </div>
            )}
            <Button asChild variant="ghost" className="h-11 w-full sm:w-auto">
              <Link to={`/cooks/${s.cook}`}>
                <ArrowLeft /> Back to {s.cook}
              </Link>
            </Button>
          </Card>
        )}

        {/* phones: the primary action sticks above the tab bar */}
        {waiting && (
          <div className="bottom-tabbar sticky z-10 -mx-4 flex gap-2 border-t border-border bg-background/95 px-4 py-3 backdrop-blur sm:-mx-5 sm:px-5 md:hidden">
            {hasPrepare && step === 1 ? (
              <Button type="button" className="h-12 flex-1 text-[15px]" onClick={() => setStep(2)}>
                Next: paste the values
              </Button>
            ) : (
              <>
                {hasPrepare && (
                  <Button type="button" variant="outline" className="h-12" onClick={() => setStep(1)} aria-label="Back to the steps">
                    <ArrowLeft />
                  </Button>
                )}
                <Button type="submit" form="connect-form" disabled={saving || !canSave} className="h-12 flex-1 text-[15px]">
                  {saving && <Loader2 className="size-4 animate-spin" />} Connect
                </Button>
              </>
            )}
          </div>
        )}
      </div>
    </PageShell>
  );
}
