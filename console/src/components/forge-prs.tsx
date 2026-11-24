import { useState, type FormEvent } from "react";
import type { PrSummary, PrView } from "@corntchap/api";
import { AlertTriangle, Check, Clock, ExternalLink, GitMerge, GitPullRequest, GitPullRequestClosed, GitPullRequestDraft, Link2, Loader2, Unlink, X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useProcCall } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { cn } from "@/lib/utils";
import { SectionLabel } from "./bits";

const chip = "inline-flex h-6 shrink-0 items-center gap-1 rounded-full border px-2 text-[11.5px] font-medium leading-none";

function stateOf(s: { state: string; draft: boolean }) {
  if (s.state === "merged") return { icon: GitMerge, label: "merged", cls: "border-success/40 bg-success/10 text-success" };
  if (s.state === "closed") return { icon: GitPullRequestClosed, label: "closed", cls: "border-border bg-muted/60 text-muted-foreground" };
  if (s.draft) return { icon: GitPullRequestDraft, label: "draft", cls: "border-border bg-muted/60 text-muted-foreground" };
  return { icon: GitPullRequest, label: "open", cls: "border-primary/40 bg-primary/10 text-primary" };
}

function checksOf(checks: string) {
  switch (checks) {
    case "success":
      return { icon: Check, label: "checks passed", cls: "border-success/40 bg-success/10 text-success" };
    case "failure":
      return { icon: X, label: "checks failed", cls: "border-destructive/40 bg-destructive/10 text-destructive" };
    case "pending":
      return { icon: Clock, label: "checks running", cls: "border-corn/50 bg-corn/15 text-corn-text" };
    default:
      return null;
  }
}

function reviewOf(review: string) {
  switch (review) {
    case "approved":
      return { label: "approved", cls: "border-success/40 bg-success/10 text-success" };
    case "changes_requested":
      return { label: "changes requested", cls: "border-destructive/40 bg-destructive/10 text-destructive" };
    case "commented":
      return { label: "commented", cls: "border-border bg-muted/60 text-muted-foreground" };
    default:
      return null;
  }
}

/** The small indicator on a board card or list row: the PR's state, how its checks went, conflicts. Nothing without a PR. */
export function PrChip({ pr }: { pr?: PrSummary | null | undefined }) {
  if (!pr) return null;
  const st = stateOf(pr);
  const ck = pr.state === "open" ? checksOf(pr.checks) : null;
  const rv = pr.state === "open" ? reviewOf(pr.review) : null;
  const more = pr.count > 1 ? ` (+${pr.count - 1})` : "";
  const title = [`${pr.repo}#${pr.number} ${st.label}`, ck?.label, rv && `review: ${rv.label}`, pr.conflict && "has conflicts"].filter(Boolean).join(", ");
  return (
    <span className="inline-flex shrink-0 items-center gap-1" title={title + more} role="img" aria-label={`Pull request: ${title}${more}`}>
      <span className={cn(chip, "gap-1 px-1.5", st.cls)}>
        <st.icon className="size-3" aria-hidden="true" />
        <span className="font-mono text-[11px]">#{pr.number}</span>
      </span>
      {ck && (
        <span className={cn(chip, "px-1.5", ck.cls)}>
          <ck.icon className="size-3" aria-hidden="true" />
        </span>
      )}
      {rv && pr.review !== "commented" && <span className={cn("size-2 rounded-full", pr.review === "approved" ? "bg-success" : "bg-destructive")} aria-hidden="true" />}
      {pr.conflict && pr.state === "open" && <AlertTriangle className="size-3.5 text-destructive" aria-hidden="true" />}
      {more && <span className="font-mono text-[11px] text-muted-foreground">{more.trim()}</span>}
    </span>
  );
}

function PrStrip({ pr, onUnlink, busy }: { pr: PrView; onUnlink: () => void; busy: boolean }) {
  const st = stateOf(pr);
  const open = pr.state === "open";
  const ck = checksOf(pr.checks);
  const rv = reviewOf(pr.review);
  return (
    <li className="rounded-lg border border-border bg-card p-3">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <a
            href={pr.url}
            target="_blank"
            rel="noreferrer"
            className="group inline-flex max-w-full items-center gap-1.5 rounded text-[14px] leading-snug font-medium underline-offset-2 hover:underline focus-visible:underline max-md:py-1"
          >
            <span className="truncate">{pr.title || `${pr.repo}#${pr.number}`}</span>
            <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
          </a>
          <p className="mt-0.5 truncate font-mono text-[11.5px] text-muted-foreground">
            {pr.repo}#{pr.number}
            {pr.head_branch && <> · {pr.head_branch}</>}
          </p>
        </div>
        <Button variant="ghost" size="sm" className="shrink-0 text-muted-foreground max-md:h-11" disabled={busy} onClick={onUnlink} aria-label={`Unlink ${pr.repo}#${pr.number}`}>
          {busy ? <Loader2 className="animate-spin" /> : <Unlink />} <span className="max-sm:sr-only">Unlink</span>
        </Button>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <span className={cn(chip, st.cls)}>
          <st.icon className="size-3.5" aria-hidden="true" /> {st.label}
        </span>
        {open && ck && (
          <span className={cn(chip, ck.cls)}>
            <ck.icon className="size-3.5" aria-hidden="true" /> {ck.label}
          </span>
        )}
        {open && rv && <span className={cn(chip, rv.cls)}>{rv.label}</span>}
        {open && pr.mergeable === "false" && (
          <span className={cn(chip, "border-destructive/40 bg-destructive/10 text-destructive")}>
            <AlertTriangle className="size-3.5" aria-hidden="true" /> has conflicts
          </span>
        )}
      </div>
      {open && pr.checks === "failure" && pr.failing.length > 0 && (
        <p className="mt-2 text-[12.5px] text-muted-foreground">
          Failing:{" "}
          {pr.checks_url ? (
            <a href={pr.checks_url} target="_blank" rel="noreferrer" className="font-mono underline-offset-2 hover:underline">
              {pr.failing.join(", ")}
            </a>
          ) : (
            <span className="font-mono">{pr.failing.join(", ")}</span>
          )}
        </p>
      )}
    </li>
  );
}

/** A task's pull requests (from the forge: README, "GitHub (and other forges)") and a way to link or unlink one by hand. Shows nothing for a task without one, but the link button. */
export function TaskPrs({ task }: { task: { id: string; prs?: PrView[] | undefined; status: string } }) {
  const call = useProcCall();
  const [adding, setAdding] = useState(false);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const prs = task.prs ?? [];
  if (task.status === "cancelled" && prs.length === 0) return null;

  async function link(e: FormEvent) {
    e.preventDefault();
    setBusy("link");
    try {
      await call("taskLinkPr", { id: task.id, url: url.trim() });
      setUrl("");
      setAdding(false);
      toast.success("Pull request linked");
    } catch (err) {
      toast.error("Could not link that pull request", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }
  async function unlink(pr: PrView) {
    setBusy(pr.url);
    try {
      await call("taskUnlinkPr", { id: task.id, url: pr.url });
      toast.success("Pull request unlinked");
    } catch (err) {
      toast.error("Could not unlink it", { description: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <section aria-labelledby="prs">
      <div className="mb-2 flex items-center justify-between gap-2">
        <SectionLabel>
          <span id="prs">Pull requests</span>
        </SectionLabel>
        {!adding && (
          <Button variant="ghost" size="sm" className="text-muted-foreground max-md:h-11" onClick={() => setAdding(true)}>
            <Link2 /> Link a PR
          </Button>
        )}
      </div>
      {prs.length === 0 && !adding && <p className="text-muted-foreground">None yet. A pull request whose branch or title names {task.id} shows up here by itself.</p>}
      {prs.length > 0 && (
        <ul className="space-y-2">
          {prs.map((p) => (
            <PrStrip key={p.url} pr={p} busy={busy === p.url} onUnlink={() => void unlink(p)} />
          ))}
        </ul>
      )}
      {adding && (
        <form onSubmit={(e) => void link(e)} className="mt-2 flex flex-col gap-2 sm:flex-row">
          <Input
            autoFocus
            required
            type="url"
            inputMode="url"
            autoCapitalize="none"
            spellCheck={false}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://github.com/acme/shop/pull/12"
            aria-label="Pull request address"
            className="h-11 font-mono text-[13px] sm:h-9"
          />
          <div className="flex gap-2">
            <Button type="submit" disabled={busy === "link" || !url.trim()} className="max-sm:h-11 max-sm:flex-1">
              {busy === "link" && <Loader2 className="animate-spin" />} Link
            </Button>
            <Button type="button" variant="ghost" className="max-sm:h-11" onClick={() => setAdding(false)}>
              Cancel
            </Button>
          </div>
        </form>
      )}
    </section>
  );
}
