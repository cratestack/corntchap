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
  BookMarked,
  Eraser,
  PartyPopper,
  RotateCcw,
  Soup,
  MessageCircleQuestion,
  CircleCheck,
  CircleX,
  Paperclip,
  FileWarning,
  MessagesSquare,
  type LucideIcon,
} from "lucide-react";
import { Link } from "react-router";
import { cn } from "@/lib/utils";
import { fmtAgo, useNow } from "@/lib/format";
import { Avatar } from "./bits";
import { useOpenMemory, useOpenTask } from "@/lib/nav";
import { fileHref } from "@/lib/files";
import { questionLink } from "./questions";

interface Described {
  icon: LucideIcon;
  tone: string;
  verb: string;
  detail?: string | undefined;
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
    case "task.unblocked":
      return { icon: Unlock, tone: "text-primary", verb: "unblocked" };
    case "task.cancelled":
      return { icon: Trash2, tone: "text-destructive", verb: "tossed", detail: p.reason ?? undefined };
    case "task.restored":
      return { icon: Undo2, tone: "text-primary", verb: "restored" };
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

const Item = memo(function Item({ e, compact, now }: { e: ChangeView; compact: boolean; now: number }) {
  const open = useOpenTask();
  const openMemory = useOpenMemory();
  const d = describe(e);
  const Icon = d.icon;
  const actor = e.actor ?? "the kitchen";
  return (
    <li className="flex gap-3 py-2.5">
      <span className="relative mt-0.5">
        <Avatar name={actor} className="size-6 text-[11px]" />
        <span className="absolute -right-1 -bottom-1 grid size-3.5 place-items-center rounded-full bg-card ring-1 ring-border">
          <Icon className={cn("size-2.5", d.tone)} aria-hidden="true" />
        </span>
      </span>
      <div className="min-w-0 flex-1">
        <p className={cn("leading-snug", compact ? "text-[12.5px]" : "text-[13.5px]")}>
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
              className="rounded font-mono text-[12px] font-bold underline-offset-2 hover:underline focus-visible:underline"
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
        <p className="mt-0.5 font-mono text-[10.5px] text-muted-foreground">
          {fmtAgo(e.at, now)}
          {!compact && <span> · {e.project}</span>}
        </p>
      </div>
    </li>
  );
});

export function EventFeed({
  events,
  compact = false,
  limit,
  empty,
}: {
  events: ChangeView[];
  compact?: boolean;
  limit?: number;
  empty?: string;
}) {
  const now = useNow();
  const shown = useMemo(() => {
    const newestFirst = [...events].reverse();
    return limit ? newestFirst.slice(0, limit) : newestFirst;
  }, [events, limit]);
  if (shown.length === 0) {
    return <p className="py-6 text-center text-muted-foreground">{empty ?? "Nothing has happened yet."}</p>;
  }
  return (
    <ul className="divide-y divide-border">
      {shown.map((e) => (
        <Item key={e.seq} e={e} compact={compact} now={now} />
      ))}
    </ul>
  );
}
