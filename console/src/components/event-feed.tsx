import { memo, useMemo } from "react";
import type { ChangeView } from "@corntchap/api";
import {
  Trash2,
  CheckCheck,
  CirclePlay,
  Flame,
  Hourglass,
  MessageSquare,
  Plus,
  Square,
  SquareCheck,
  Undo2,
  Unlock,
  Pencil,
  UserRound,
  BookMarked,
  Eraser,
  PartyPopper,
  RotateCcw,
  Soup,
  MessageCircleQuestion,
  ClipboardCheck,
  CircleCheck,
  CircleX,
  Paperclip,
  FileWarning,
  MessagesSquare,
  GitBranch,
  GitMerge,
  GitPullRequest,
  AlertTriangle,
  Loader2,
  type LucideIcon,
} from "lucide-react";
import { Link } from "react-router";
import { cn } from "@/lib/utils";
import { fmtAgo, useNow } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Avatar } from "./bits";
import { useOpenMemory, useOpenTask } from "@/lib/nav";
import { fileHref } from "@/lib/files";
import { questionLink } from "./questions";
import { VirtualGroupedList, type VirtualGroup } from "./ui/virtual-grouped-list";

interface Described {
  icon: LucideIcon;
  tone: string;
  verb: string;
  detail?: string | undefined;
}

/** `acme/shop#12` from a forge event's payload. */
function forgeWord(p: { repo?: string | null | undefined; number?: number | null | undefined }): string {
  return p.repo && p.number ? `${p.repo}#${p.number}` : "";
}

export function describe(e: ChangeView): Described {
  const p = e.payload ?? {};
  switch (e.kind) {
    case "task.created":
      return { icon: Plus, tone: "text-primary", verb: "added", detail: p.title ?? undefined };
    case "task.claimed":
      return { icon: Flame, tone: "text-corn-text", verb: "put on the fire", detail: p.epoch ? `epoch ${p.epoch}` : undefined };
    case "item.checked":
      return {
        icon: p.done === false ? Square : SquareCheck,
        tone: "text-success",
        verb: p.done === false ? "unticked item" : "ticked item",
        detail: p.position ? `#${p.position}` : undefined,
      };
    case "task.commented":
      return { icon: MessageSquare, tone: "text-muted-foreground", verb: "commented on" };
    case "task.completed":
      return { icon: CheckCheck, tone: "text-success", verb: "served", detail: p.result ?? undefined };
    case "task.released":
      return { icon: Undo2, tone: "text-muted-foreground", verb: "put back", detail: p.reason ?? undefined };
    case "task.blocked":
      return { icon: Hourglass, tone: "text-waiting", verb: p.question ? "parked" : "stalled", detail: p.reason ?? undefined };
    case "question.asked":
      return { icon: MessageCircleQuestion, tone: "text-primary", verb: "asked", detail: p.title ?? undefined };
    case "question.answered":
      return { icon: CircleCheck, tone: "text-success", verb: "answered", detail: p.answer ?? undefined };
    case "question.dismissed":
      return { icon: CircleX, tone: "text-muted-foreground", verb: "dismissed", detail: p.answer ?? undefined };
    case "question.noted":
      return { icon: MessageSquare, tone: "text-muted-foreground", verb: "added a note to", detail: p.title ?? undefined };
    case "review.requested":
      return { icon: ClipboardCheck, tone: "text-primary", verb: p.reviewer ? `asked ${p.reviewer} to review` : "asked the team to review", detail: [p.review_id, p.round && p.round > 1 ? `round ${p.round}` : null, p.note].filter(Boolean).join(" · ") || undefined };
    case "review.done":
      return {
        icon: p.verdict === "approved" ? CircleCheck : p.verdict === "changes_requested" ? CircleX : MessageSquare,
        tone: p.verdict === "approved" ? "text-success" : p.verdict === "changes_requested" ? "text-destructive" : "text-muted-foreground",
        verb: p.verdict === "approved" ? "approved" : p.verdict === "changes_requested" ? "asked for changes on" : "commented on the review of",
        detail: [p.review_id, p.excerpt].filter(Boolean).join(" · ") || undefined,
      };
    case "review.cancelled":
      return { icon: CircleX, tone: "text-muted-foreground", verb: "withdrew the review request on", detail: p.review_id ?? undefined };
    case "task.unblocked":
      return { icon: Unlock, tone: "text-primary", verb: "unblocked" };
    case "task.cancelled":
      return { icon: Trash2, tone: "text-destructive", verb: "tossed", detail: p.reason ?? undefined };
    case "task.restored":
      return { icon: Undo2, tone: "text-primary", verb: "restored" };
    case "task.owner_changed":
      return {
        icon: UserRound,
        tone: "text-primary",
        verb: p.new_owner ? `made ${p.new_owner} the owner of` : "cleared the owner of",
        detail: p.old_owner ? `was ${p.old_owner}` : undefined,
      };
    case "task.updated":
      return { icon: Pencil, tone: "text-muted-foreground", verb: "edited" };
    case "memory.saved":
      return {
        icon: BookMarked,
        tone: "text-success",
        verb: p.created ? "stocked the pantry with" : "updated in the pantry",
        detail: p.title ?? undefined,
      };
    case "memory.forgotten":
      return { icon: Eraser, tone: "text-destructive", verb: "forgot", detail: p.title ?? undefined };
    case "menu.created":
      return { icon: Soup, tone: "text-corn-text", verb: "put a new" };
    case "menu.updated":
      return { icon: Pencil, tone: "text-muted-foreground", verb: "edited the" };
    case "menu.served":
      return { icon: PartyPopper, tone: "text-success", verb: "served the" };
    case "menu.reopened":
      return { icon: RotateCcw, tone: "text-primary", verb: "reopened the" };
    case "file.created":
      return { icon: Paperclip, tone: "text-primary", verb: "uploaded", detail: undefined };
    case "file.ready":
      return { icon: CircleCheck, tone: "text-success", verb: "finished uploading", detail: undefined };
    case "file.failed":
      return { icon: FileWarning, tone: "text-destructive", verb: "upload failed:", detail: p.reason ?? undefined };
    case "file.deleted":
      return { icon: Trash2, tone: "text-destructive", verb: "deleted", detail: undefined };
    case "pass.posted":
      return {
        icon: MessagesSquare,
        tone: "text-primary",
        verb: `${p.via ? `(via ${p.via}) ` : ""}${p.to ? `wrote to ${p.to}` : "posted on the pass"}`,
        detail: p.title ?? undefined,
      };
    case "task.propagated":
      return { icon: GitBranch, tone: "text-primary", verb: "passed an upstream change on", detail: p.upstream ? `from ${p.upstream}` : undefined };
    case "project.repo_added":
      return { icon: GitBranch, tone: "text-primary", verb: "added the repository", detail: p.name ?? undefined };
    case "project.repo_removed":
      return { icon: GitBranch, tone: "text-destructive", verb: "removed the repository", detail: p.name ?? undefined };
    case "project.repo_linked":
      return { icon: GitBranch, tone: "text-muted-foreground", verb: "linked repositories", detail: p.repo && p.depends_on ? `${p.repo} depends on ${p.depends_on}` : undefined };
    case "forge.linked":
      return { icon: GitPullRequest, tone: "text-primary", verb: p.via === "explicit" ? "linked a pull request to" : "found the pull request of", detail: forgeWord(p) };
    case "forge.unlinked":
      return { icon: GitPullRequest, tone: "text-muted-foreground", verb: "unlinked a pull request from" };
    case "forge.pr":
      return {
        icon: GitPullRequest,
        tone: "text-primary",
        verb: p.action === "opened" ? "opened a pull request on" : p.action === "reopened" ? "reopened the pull request of" : p.action === "closed" ? "closed the pull request of" : "pushed to the pull request of",
        detail: forgeWord(p),
      };
    case "forge.merged":
      return { icon: GitMerge, tone: "text-success", verb: "merged the pull request of", detail: `${forgeWord(p)} · status unchanged` };
    case "forge.checks":
      return {
        icon: p.conclusion === "success" ? CircleCheck : CircleX,
        tone: p.conclusion === "success" ? "text-success" : "text-destructive",
        verb: p.conclusion === "success" ? "checks passed on" : `checks ${String(p.conclusion ?? "failed").replace("_", " ")} on`,
        detail: [forgeWord(p), p.failing].filter(Boolean).join(" · "),
      };
    case "forge.review": {
      const who = p.author ?? "someone";
      return {
        icon: p.review === "approved" ? CircleCheck : p.review === "changes_requested" ? CircleX : MessageSquare,
        tone: p.review === "approved" ? "text-success" : p.review === "changes_requested" ? "text-destructive" : "text-muted-foreground",
        verb: p.review === "approved" ? `${who} approved` : p.review === "changes_requested" ? `${who} asked for changes on` : `${who} reviewed`,
        detail: [forgeWord(p), p.excerpt].filter(Boolean).join(" · "),
      };
    }
    case "forge.comment":
      return {
        icon: MessageSquare,
        tone: "text-muted-foreground",
        verb: `${p.author ?? "someone"} commented on`,
        detail: [forgeWord(p), p.path ? `${p.path}${p.line ? `:${p.line}` : ""}` : "", p.excerpt].filter(Boolean).join(" · "),
      };
    case "forge.conflict":
      return { icon: AlertTriangle, tone: "text-destructive", verb: "found merge conflicts on", detail: forgeWord(p) };
    case "task.lease_expired":
      return { icon: Hourglass, tone: "text-corn-text", verb: "lease ran out on" };
    default:
      return { icon: CirclePlay, tone: "text-muted-foreground", verb: e.kind.replace(/[._]/g, " ") };
  }
}

/** The file's name, linking to its permanent URL unless it is gone or not there yet. */
function FileLink({ e }: { e: ChangeView }) {
  const id = e.payload.file ?? "";
  const name = e.payload.title ?? id;
  const cls = "inline-block max-w-[16rem] truncate align-bottom font-mono text-[12px] font-bold";
  if (id && e.kind !== "file.deleted" && e.kind !== "file.failed" && e.payload.title) {
    return (
      <a
        href={fileHref(id, e.payload.title)}
        target="_blank"
        rel="noreferrer"
        title={name}
        className={cn(cls, "underline-offset-2 hover:underline focus-visible:underline")}
      >
        {name}
      </a>
    );
  }
  return (
    <span title={name} className={cn(cls, e.kind === "file.deleted" && "line-through decoration-muted-foreground/60")}>
      {name}
    </span>
  );
}

/** The relative time of an event. It subscribes to the clock itself, so a tick re-renders only these few spans and not the rows. */
function Ago({ at }: { at: string }) {
  return <>{fmtAgo(at, useNow())}</>;
}

const Item = memo(function Item({ e, compact, last }: { e: ChangeView; compact: boolean; last: boolean }) {
  const open = useOpenTask();
  const openMemory = useOpenMemory();
  const d = describe(e);
  const Icon = d.icon;
  const actor = e.actor ?? (e.kind.startsWith("forge.") ? "GitHub" : "the kitchen");
  return (
    <div className={cn("relative flex gap-3", compact ? "py-2.5" : "py-3")}>
      {!compact && !last && <span aria-hidden="true" className="absolute top-11 -bottom-1 left-3 w-px bg-border" />}
      <span className="relative mt-0.5 self-start">
        <Avatar name={actor} className="size-6 text-[11px]" />
        <span className="absolute -right-1 -bottom-1 grid size-3.5 place-items-center rounded-full bg-card ring-1 ring-border">
          <Icon className={cn("size-2.5", d.tone)} aria-hidden="true" />
        </span>
      </span>
      <div className="min-w-0 flex-1">
        <p className={cn("leading-snug", compact ? "text-[12.5px]" : "text-[14.5px] md:text-[13.5px]")}>
          <span className="font-semibold">{actor}</span> <span className="text-muted-foreground">{d.verb}</span>{" "}
          {e.kind.startsWith("question.") && e.payload.question && (
            <>
              <Link
                to={questionLink(e.project, e.payload.question)}
                className="rounded font-mono text-[12px] font-bold underline-offset-2 hover:underline focus-visible:underline"
              >
                {e.payload.question}
              </Link>
              {e.kind === "question.answered" && e.payload.relayed_by && (
                <span className="text-muted-foreground"> (via {e.payload.relayed_by})</span>
              )}
              {e.task && <span className="text-muted-foreground"> on </span>}
            </>
          )}
          {e.task && (
            <button
              type="button"
              onClick={() => open(e.task!)}
              className="rounded font-mono text-[12px] font-bold underline-offset-2 hover:underline focus-visible:underline max-md:py-1"
            >
              {e.task}
            </button>
          )}
          {e.kind.startsWith("menu.") && (
            <>
              <Link
                to={`/pots/${e.project}/board?menu=${encodeURIComponent(e.payload.menu ?? "")}`}
                className="rounded font-semibold underline-offset-2 hover:underline focus-visible:underline"
              >
                {e.payload.title ?? e.payload.menu}
              </Link>{" "}
              <span className="text-muted-foreground">menu</span>
            </>
          )}
          {e.kind.startsWith("file.") && (e.payload.file || e.payload.title) && (
            <>
              {e.task && <span className="text-muted-foreground"> · </span>}
              <FileLink e={e} />
            </>
          )}
          {e.kind === "memory.saved" && e.payload.key && (
            <button
              type="button"
              onClick={() => openMemory(e.project, e.payload.key!)}
              className="ml-1 rounded font-mono text-[12px] font-bold underline-offset-2 hover:underline focus-visible:underline"
            >
              {e.payload.key}
            </button>
          )}
          {e.kind === "memory.forgotten" && e.payload.key && (
            <span className="ml-1 font-mono text-[12px] font-bold line-through decoration-muted-foreground/60">{e.payload.key}</span>
          )}
          {d.detail && <span className="text-muted-foreground"> · {d.detail}</span>}
        </p>
        <p className="mt-1 font-mono text-[10.5px] text-muted-foreground md:mt-0.5">
          <Ago at={e.at} />
          {!compact && <span> · {e.project}</span>}
        </p>
      </div>
    </div>
  );
});

/** One event of the full feed, and whether it closes its day (the timeline line stops there). */
interface Row {
  e: ChangeView;
  last: boolean;
}

/** Newest-first events cut into days; a group's `label` is the ISO time of its first event, shown by `DayLabel`. */
function byDay(shown: ChangeView[]): { groups: VirtualGroup[]; rows: Row[] } {
  const groups: VirtualGroup[] = [];
  const rows: Row[] = [];
  shown.forEach((e, i) => {
    const day = dayKey(e.at);
    const first = i === 0 || day !== dayKey(shown[i - 1]!.at);
    if (first) groups.push({ key: day, label: e.at, count: 0 });
    groups[groups.length - 1]!.count++;
    rows.push({ e, last: i === shown.length - 1 || day !== dayKey(shown[i + 1]!.at) });
  });
  return { groups, rows };
}

function DayLabel({ at }: { at: string }) {
  return <>{dayLabel(at, useNow())}</>;
}

export function EventFeed({
  events,
  compact = false,
  limit,
  empty,
  hasOlder = false,
  loadingOlder = false,
  onLoadOlder,
}: {
  events: ChangeView[];
  compact?: boolean;
  limit?: number;
  empty?: string;
  /** The full feed: older events than the oldest shown exist, and `onLoadOlder` reads them (also when the end scrolls into view). */
  hasOlder?: boolean;
  loadingOlder?: boolean;
  onLoadOlder?: () => void;
}) {
  const shown = useMemo(() => {
    const newestFirst = [...events].reverse();
    return limit ? newestFirst.slice(0, limit) : newestFirst;
  }, [events, limit]);
  const days = useMemo(() => (compact ? null : byDay(shown)), [compact, shown]);
  if (shown.length === 0) {
    return <p className="py-6 text-center text-muted-foreground">{empty ?? "Nothing has happened yet."}</p>;
  }
  if (!days) {
    // the compact feeds hold a handful of events (8 to 14): no need to virtualize
    return (
      <ul className="divide-y divide-border">
        {shown.map((e, i) => (
          <li key={e.seq}>
            <Item e={e} compact last={i === shown.length - 1} />
          </li>
        ))}
      </ul>
    );
  }
  // full feed: a timeline with a sticky heading per day, virtualized against the page scroll
  return (
    <VirtualGroupedList
      scroll="window"
      aria-label="Activity"
      groups={days.groups}
      items={days.rows}
      itemKey={(r) => r.e.seq}
      defaultItemHeight={64}
      onEndReached={hasOlder && !loadingOlder ? onLoadOlder : undefined}
      footer={
        hasOlder ? (
          <div className="flex justify-center py-4">
            <Button type="button" variant="outline" size="sm" disabled={loadingOlder} onClick={onLoadOlder}>
              {loadingOlder && <Loader2 className="animate-spin" />}
              {loadingOlder ? "Loading older activity" : "Load older activity"}
            </Button>
          </div>
        ) : null
      }
      stickyClassName="top-[calc(3.5rem+env(safe-area-inset-top)+1px)]! md:top-0!"
      renderGroup={(g) => (
        <div className="label-caps border-b border-border bg-card pt-4 pb-2" role="heading" aria-level={3}>
          <DayLabel at={g.label} />
        </div>
      )}
      renderItem={(r) => <Item e={r.e} compact={false} last={r.last} />}
    />
  );
}

const dayKey = (iso: string) => new Date(iso).toDateString();
function dayLabel(iso: string, now: number): string {
  const d = new Date(iso);
  const today = new Date(now);
  const yesterday = new Date(now - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}
