import type { ReactNode } from "react";
import { Link } from "react-router";
import type { WorkComment, WorkQuestion, WorkReview, WorkTask } from "@corntchap/api";
import { CircleHelp, ClipboardCheck, Flame, GitPullRequest, MessageSquare, Hourglass } from "lucide-react";
import { PageShell } from "@/components/page";
import { Avatar, BUCKET_DOT, EmptyState, IdChip, RowSkeleton } from "@/components/bits";
import { PrChip } from "@/components/forge-prs";
import { ReviewChip } from "@/components/reviews";
import { useMyWork } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, plural, useNow } from "@/lib/format";
import { useOpenTask } from "@/lib/nav";
import { useSession } from "@/lib/session";
import { cn } from "@/lib/utils";

const POT_CHIP = "inline-flex shrink-0 items-center rounded-md bg-muted px-1.5 py-px font-mono text-[10.5px] font-bold text-muted-foreground";

function PotChip({ pot }: { pot: string }) {
  return <span className={POT_CHIP}>{pot}</span>;
}

const dotOf = (status: string) => (status === "claimed" ? BUCKET_DOT.claimed : status === "blocked" ? BUCKET_DOT.waiting : BUCKET_DOT.ready);

const ROW = "flex min-h-14 w-full items-start gap-3 px-3.5 py-3 text-left transition-colors hover:bg-muted/50 active:bg-muted/70 focus-visible:bg-muted/50 focus-visible:outline-none";

function Rows({ children }: { children: ReactNode }) {
  return <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{children}</div>;
}

function Section({ id, title, count, hint, children }: { id: string; title: string; count: number; hint: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="scroll-mt-20">
      <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
        <h2 id={id} className="m-0 font-display text-[19px]">
          {title}
        </h2>
        <span className="font-mono text-[12px] text-muted-foreground">{count}</span>
        <span className="text-[12.5px] text-muted-foreground">{hint}</span>
      </div>
      <div className="mt-2.5 space-y-4">{children}</div>
    </section>
  );
}

function Empty({ title, children }: { title: string; children: ReactNode }) {
  return (
    <EmptyState band={false} title={title} className="border-dashed bg-transparent">
      {children}
    </EmptyState>
  );
}

function TaskRow({ t, onOpen, me, showPot = true }: { t: WorkTask; onOpen: (id: string) => void; me: string; showPot?: boolean }) {
  const now = useNow();
  const live = t.status === "claimed" && t.lease_live;
  return (
    <button type="button" onClick={() => onOpen(t.id)} aria-label={`${t.id}: ${t.title}`} className={ROW}>
      <span className={cn("mt-[7px] size-2.5 shrink-0 rounded-full", dotOf(t.status), live && "live-dot")} aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="flex items-start gap-2">
          <span className="line-clamp-2 min-w-0 flex-1 text-[15px] leading-snug font-medium">{t.title}</span>
          {t.priority >= 2 && <Flame className="mt-1 size-3.5 shrink-0 text-primary" aria-label="urgent" />}
        </span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={t.id} className="text-[11.5px] font-normal text-muted-foreground" />
          {showPot && <PotChip pot={t.project} />}
          <span>{t.status === "claimed" ? "on the fire" : t.status === "blocked" ? "blocked" : "open"}</span>
          {t.holder && t.holder !== me && (
            <span className="inline-flex items-center gap-1" title={`Held by ${t.holder}`}>
              <Avatar name={t.holder} className="size-4 rounded-[5px] text-[9px]" />
              <span className="max-w-24 truncate">{t.holder}</span>
            </span>
          )}
          <PrChip pr={t.pr} />
          <ReviewChip review={t.review} />
          {t.because && (
            <span className="font-mono text-[11px]">{t.via === "question" ? `asks you ${t.because}` : `waits for ${t.because}`}</span>
          )}
          <span className="ml-auto font-mono text-[11px] tabular-nums">{fmtAgo(t.updated_at, now)}</span>
        </span>
      </span>
    </button>
  );
}

function QuestionRow({ q }: { q: WorkQuestion }) {
  const now = useNow();
  return (
    <Link to={`/pots/${q.project}/questions?question=${encodeURIComponent(q.id)}`} className={ROW}>
      <CircleHelp className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 text-[15px] leading-snug font-medium">{q.title}</span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={q.id} className="text-[11.5px] font-normal text-muted-foreground" />
          <PotChip pot={q.project} />
          {q.from && <span>from {q.from}</span>}
          {q.task && <span className="font-mono text-[11px]">on {q.task}</span>}
          <span className="ml-auto font-mono text-[11px] tabular-nums">{fmtAgo(q.at, now)}</span>
        </span>
      </span>
    </Link>
  );
}

function ReviewRow({ r, onOpen }: { r: WorkReview; onOpen: (id: string) => void }) {
  const now = useNow();
  return (
    <button type="button" onClick={() => onOpen(r.task)} className={ROW} aria-label={`Review ${r.id} of ${r.task}`}>
      <ClipboardCheck className="mt-0.5 size-4 shrink-0 text-corn-text" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 text-[15px] leading-snug font-medium">{r.task_title ?? r.task}</span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={r.task} className="text-[11.5px] font-normal text-muted-foreground" />
          <PotChip pot={r.project} />
          <span>review, round {r.round}</span>
          {r.from && <span>asked by {r.from}</span>}
          {r.note && <span className="min-w-0 max-w-full truncate">{r.note}</span>}
          <span className="ml-auto font-mono text-[11px] tabular-nums">{fmtAgo(r.at, now)}</span>
        </span>
      </span>
    </button>
  );
}

function CommentRow({ c, onOpen }: { c: WorkComment; onOpen: (id: string) => void }) {
  const now = useNow();
  return (
    <button type="button" onClick={() => onOpen(c.task)} className={ROW} aria-label={`Comment on ${c.task}`}>
      <MessageSquare className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 text-[14px] leading-snug">{c.excerpt}</span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={c.task} className="text-[11.5px] font-normal text-muted-foreground" />
          <PotChip pot={c.project} />
          {c.author && <span>{c.author} wrote</span>}
          <span className="ml-auto font-mono text-[11px] tabular-nums">{fmtAgo(c.at, now)}</span>
        </span>
      </span>
    </button>
  );
}

export function MyWorkPage() {
  const q = useMyWork();
  const me = useSession();
  const open = useOpenTask();
  const w = q.data;

  return (
    <PageShell
      title="My work"
      eyebrow={`@${me.name}`}
      sub="What you answer for, what you are cooking and what waits on you, across every pot you can see."
    >
      {q.error && !w ? (
        <EmptyState band={false} title="Could not read your work">
          {errorMessage(q.error)}
        </EmptyState>
      ) : !w ? (
        <RowSkeleton rows={5} />
      ) : (
        <div className="mx-auto max-w-3xl space-y-9 md:mx-0">
          <nav aria-label="Sections" className="flex flex-wrap gap-2">
            {(
              [
                ["owned", "Owned", w.counts.owned],
                ["holding", "Holding", w.counts.holding],
                ["waiting", "Waiting on me", w.counts.waiting],
                ["blocked", "Blocked on me", w.counts.blocked],
              ] as const
            ).map(([id, label, n]) => (
              <a
                key={id}
                href={`#my-${id}`}
                className={cn(
                  "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-[12.5px] transition-colors hover:border-foreground/30",
                  n > 0 && id === "waiting" ? "border-primary/40 bg-primary/10 text-primary" : "border-border bg-card",
                )}
              >
                {label} <span className="font-mono text-[11.5px]">{n}</span>
              </a>
            ))}
          </nav>

          <Section id="my-owned" title="Owned" count={w.counts.owned} hint="Tasks you answer for, whoever does the work.">
            {w.owned.length === 0 ? (
              <Empty title="Nothing is yours yet">
                A task lands here when someone makes you its owner: set the Owner on a task, or create it with an owner.
              </Empty>
            ) : (
              w.owned.map((pot) => (
                <div key={pot.project}>
                  <h3 className="mb-1.5 flex items-center gap-2 text-[13px] font-normal text-muted-foreground" style={{ fontFamily: "var(--font-sans)", letterSpacing: 0 }}>
                    <PotChip pot={pot.project} /> <span className="truncate">{pot.name}</span>
                    <span className="font-mono text-[11px]">{pot.tasks.length}</span>
                  </h3>
                  <Rows>
                    {pot.tasks.map((t) => (
                      <TaskRow key={t.id} t={t} onOpen={open} me={me.name} showPot={false} />
                    ))}
                  </Rows>
                </div>
              ))
            )}
          </Section>

          <Section id="my-holding" title="Holding" count={w.counts.holding} hint="Tasks you hold with a live lease.">
            {w.holding.length === 0 ? (
              <Empty title="Nothing in your hands">Serve or claim a task from a board and it shows here while the lease is live.</Empty>
            ) : (
              <Rows>
                {w.holding.map((t) => (
                  <TaskRow key={t.id} t={t} onOpen={open} me={me.name} />
                ))}
              </Rows>
            )}
          </Section>

          <Section id="my-waiting" title="Waiting on me" count={w.counts.waiting} hint="Questions to you, reviews asked of you, comments to answer.">
            {w.counts.waiting === 0 ? (
              <Empty title="Nobody is waiting on you">Questions addressed to you and reviews asked of you (or of the team) land here.</Empty>
            ) : (
              <Rows>
                {w.waiting_on_me.questions.map((x) => (
                  <QuestionRow key={x.id} q={x} />
                ))}
                {w.waiting_on_me.reviews.map((r) => (
                  <ReviewRow key={r.id} r={r} onOpen={open} />
                ))}
                {w.waiting_on_me.comments.map((c, i) => (
                  <CommentRow key={`${c.task}-${i}`} c={c} onOpen={open} />
                ))}
              </Rows>
            )}
            {w.waiting_on_me.open_pool_total > 0 && (
              <div>
                <h3 className="mb-1.5 flex items-center gap-2 text-[13px] font-normal text-muted-foreground" style={{ fontFamily: "var(--font-sans)", letterSpacing: 0 }}>
                  <Hourglass className="size-3.5" aria-hidden="true" />
                  Open questions nobody picked
                  <span className="font-mono text-[11px]">{w.waiting_on_me.open_pool_total}</span>
                </h3>
                <Rows>
                  {w.waiting_on_me.open_pool.map((x) => (
                    <QuestionRow key={x.id} q={x} />
                  ))}
                </Rows>
                {w.waiting_on_me.open_pool_total > w.waiting_on_me.open_pool.length && (
                  <p className="mt-1.5 text-[12.5px] text-muted-foreground">
                    {plural(w.waiting_on_me.open_pool_total - w.waiting_on_me.open_pool.length, "more question")} in your pots: open a pot's Questions tab.
                  </p>
                )}
              </div>
            )}
          </Section>

          <Section id="my-blocked" title="Blocked on me" count={w.counts.blocked} hint="Work held up by an answer from you or by a task you own.">
            {w.blocked_on_me.length === 0 ? (
              <Empty title="You are not holding anyone up">
                A task parked on a question you must answer, or one that depends on a task you own that is not done, shows here.
              </Empty>
            ) : (
              <Rows>
                {w.blocked_on_me.map((t) => (
                  <TaskRow key={`${t.id}-${t.because}`} t={t} onOpen={open} me={me.name} />
                ))}
              </Rows>
            )}
            {w.counts.blocked > w.blocked_on_me.length && (
              <p className="mt-1.5 flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
                <GitPullRequest className="size-3.5" aria-hidden="true" /> Showing the {w.blocked_on_me.length} most recent of {w.counts.blocked}.
              </p>
            )}
          </Section>
        </div>
      )}
    </PageShell>
  );
}
