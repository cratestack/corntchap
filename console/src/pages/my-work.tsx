import type { Key, ReactNode } from "react";
import { Link } from "react-router";
import type { QuestionView, WorkComment, WorkQuestion, WorkReview, WorkTask } from "@corntchap/api";
import { CircleHelp, ClipboardCheck, Flame, Loader2, MessageSquare, Hourglass } from "lucide-react";
import { PageShell } from "@/components/page";
import { Avatar, BUCKET_DOT, EmptyState, IdChip, RowSkeleton } from "@/components/bits";
import { PrChip } from "@/components/forge-prs";
import { ShowMore, loadMore, type MoreMode } from "@/components/list-footer";
import { ReviewChip } from "@/components/reviews";
import { VirtualCombobox } from "@/components/ui/virtual-combobox";
import { VirtualList } from "@/components/ui/virtual-list";
import { useMyWork } from "@/lib/data";
import { errorMessage } from "@/lib/api";
import { fmtAgo, useNow } from "@/lib/format";
import { useListQuery } from "@/lib/list-query";
import type { ListSource } from "@/lib/listbox";
import { useOpenTask } from "@/lib/nav";
import { useMyQuestionsSource, useMyWorkCommentsSource, useMyWorkQuestionsSource, useMyWorkReviewsSource, usePotsSource } from "@/lib/sources";
import { useStoreValue } from "@/lib/use-store-value";
import { useUrlFilters } from "@/lib/url-filters";
import { useSession } from "@/lib/session";
import { cn } from "@/lib/utils";

const POT_CHIP = "inline-flex shrink-0 items-center rounded-md bg-muted px-1.5 py-px font-mono text-[10.5px] font-bold text-muted-foreground";

function PotChip({ pot }: { pot: string }) {
  return <span className={POT_CHIP}>{pot}</span>;
}

const dotOf = (status: string) => (status === "claimed" ? BUCKET_DOT.claimed : status === "blocked" ? BUCKET_DOT.waiting : BUCKET_DOT.ready);

const ROW = "flex min-h-14 w-full items-start gap-3 px-3.5 py-3 text-left transition-colors hover:bg-muted/50 active:bg-muted/70 focus-visible:bg-muted/50 focus-visible:outline-none";

/**
 * The rows of a section read from a page source: virtualized, carried by the page. The page's last list pages as its
 * end comes into view (`more: "scroll"`); every list above another section reads on a "Show more" button.
 */
function WorkList<T>({ source, itemKey, renderItem, empty, label, more = "button" }: { source: ListSource<T>; itemKey: (item: T, index: number) => Key; renderItem: (item: T) => ReactNode; empty: ReactNode; label: string; more?: MoreMode }) {
  if (source.isLoading && source.items.length === 0) return <RowSkeleton rows={3} />;
  if (source.error && source.items.length === 0) {
    return (
      <EmptyState band={false} title={`Could not read ${label.toLowerCase()}`}>
        {errorMessage(source.error)}
      </EmptyState>
    );
  }
  if (source.items.length === 0) return <>{empty}</>;
  const footer = source.isFetchingNextPage && more === "scroll" ? (
    <p role="status" className="flex items-center justify-center gap-2 py-3 text-[12.5px] text-muted-foreground">
      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> Loading more
    </p>
  ) : more === "button" && source.hasNextPage && !source.capped ? (
    <ShowMore source={source} />
  ) : source.capped ? (
    <p className="py-3 text-center text-[12.5px] text-muted-foreground">That is as far as this goes. Narrow it to one pot to see more.</p>
  ) : null;
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-card">
      <VirtualList<T>
        items={source.items}
        itemKey={itemKey}
        scroll="window"
        onEndReached={loadMore(source, more)}
        footer={footer}
        itemClassName="border-b border-border last:border-b-0"
        defaultItemHeight={72}
        aria-label={label}
        renderItem={(item) => renderItem(item)}
      />
    </div>
  );
}

/** One section of "my work" (myWorkPage): a page of tasks, re-read from the first page when tasks change. */
function useWorkSection(section: "owned" | "holding" | "blocked", project: string | null) {
  return useListQuery("myWorkPage", { section, project }, { refreshLoaded: true });
}

function Section({ id, title, count, hint, children }: { id: string; title: string; count: number | string; hint: string; children: ReactNode }) {
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

/** One kind of what waits on me: its heading and its paged rows, nothing at all while it has none. */
function WaitingList<T>({ title, icon: Icon, source, itemKey, renderItem }: { title: string; icon: typeof CircleHelp; source: ListSource<T>; itemKey: (item: T) => Key; renderItem: (item: T) => ReactNode }) {
  if (!source.isLoading && !source.error && source.items.length === 0) return null;
  return (
    <div>
      <h3 className="mb-1.5 flex items-center gap-2 text-[13px] font-sans font-normal tracking-normal text-muted-foreground">
        <Icon className="size-3.5" aria-hidden="true" />
        {title}
      </h3>
      <WorkList source={source} label={title} itemKey={itemKey} renderItem={renderItem} empty={null} />
    </div>
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

function AskedRow({ q }: { q: QuestionView }) {
  const now = useNow();
  return (
    <Link to={`/pots/${q.project}/questions?question=${encodeURIComponent(q.id)}`} className={ROW}>
      <CircleHelp className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 text-[15px] leading-snug font-medium">{q.title}</span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-muted-foreground">
          <IdChip id={q.id} className="text-[11.5px] font-normal text-muted-foreground" />
          <PotChip pot={q.project} />
          <span>{q.to ? `asked of ${q.to}` : "asked of anyone"}</span>
          {q.task && <span className="font-mono text-[11px]">on {q.task}</span>}
          <span className="ml-auto font-mono text-[11px] tabular-nums">{fmtAgo(q.created_at, now)}</span>
        </span>
      </span>
    </Link>
  );
}

/** The one filter: a pot, in the URL (`?pot=PAY`). */
const FILTERS = { pot: { type: "string" as const } };

export function MyWorkPage() {
  const me = useSession();
  const open = useOpenTask();
  const filters = useUrlFilters(FILTERS);
  const pot = filters.values.pot || null;
  const potQuery = useStoreValue("");
  const pots = usePotsSource({ q: potQuery.value });
  const summary = useMyWork(pot ?? undefined);
  const w = summary.data;
  const owned = useWorkSection("owned", pot);
  const holding = useWorkSection("holding", pot);
  const blocked = useWorkSection("blocked", pot);

  const potArg = pot ? { project: pot } : {};
  const asked = useMyQuestionsSource({ status: "open", ...potArg });
  const questions = useMyWorkQuestionsSource(potArg);
  const reviews = useMyWorkReviewsSource(potArg);
  const comments = useMyWorkCommentsSource(potArg);
  const poolTotal = w?.waiting_on_me.open_pool_total ?? 0;
  const pool = useMyWorkQuestionsSource({ ...potArg, pool: true, enabled: poolTotal > 0 });
  const waitingSources = [questions, reviews, comments];
  const nobodyWaits = waitingSources.every((x) => !x.isLoading && !x.error && x.items.length === 0);
  const taskRow = (t: WorkTask, showPot = true) => <TaskRow t={t} onOpen={open} me={me.name} showPot={showPot} />;
  const nums = { owned: w?.counts.owned ?? owned.total ?? "…", holding: w?.counts.holding ?? holding.total ?? "…", waiting: w?.counts.waiting ?? "…", blocked: w?.counts.blocked ?? blocked.total ?? "…", asked: asked.total ?? (asked.hasNextPage ? `${asked.items.length}+` : asked.items.length) };

  return (
    <PageShell
      title="My work"
      eyebrow={`@${me.name}`}
      sub="What you answer for, what you are cooking and what waits on you, across every pot you can see."
    >
      <div className="mx-auto max-w-3xl space-y-9 md:mx-0">
        <nav aria-label="Sections" className="flex flex-wrap items-center gap-2">
          {(
            [
              ["owned", "Owned"],
              ["holding", "Holding"],
              ["waiting", "Waiting on me"],
              ["blocked", "Blocked on me"],
              ["asked", "Asked by me"],
            ] as const
          ).map(([id, label]) => (
            <a
              key={id}
              href={`#my-${id}`}
              className={cn(
                "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-[12.5px] transition-colors hover:border-foreground/30",
                id === "waiting" && (w?.counts.waiting ?? 0) > 0 ? "border-primary/40 bg-primary/10 text-primary" : "border-border bg-card",
              )}
            >
              {label} <span className="font-mono text-[11.5px]">{nums[id]}</span>
            </a>
          ))}
          <VirtualCombobox<(typeof pots.items)[number]>
            label="Pot"
            trigger="pill"
            source={pots}
            onQuery={potQuery.set}
            itemKey={(x) => x.key}
            itemText={(x) => x.key}
            renderItem={(x) => (
              <span className="flex min-w-0 items-baseline gap-2">
                <span className="font-mono text-[12px] font-bold">{x.key}</span>
                <span className="truncate text-[13px] text-muted-foreground">{x.name}</span>
              </span>
            )}
            nullOption="Every pot"
            value={pot}
            onChange={(v) => filters.set({ pot: v ?? "" })}
          />
        </nav>

        {!!summary.error && !w && (
          <EmptyState band={false} title="Could not read what is waiting on you">
            {errorMessage(summary.error)}
          </EmptyState>
        )}

        <Section id="my-owned" title="Owned" count={nums.owned} hint="Tasks you answer for, whoever does the work.">
          <WorkList
            source={owned}
            label="Owned tasks"
            itemKey={(t) => t.id}
            renderItem={(t) => taskRow(t)}
            empty={
              <Empty title="Nothing is yours yet">
                A task lands here when someone makes you its owner: set the Owner on a task, or create it with an owner.
              </Empty>
            }
          />
        </Section>

        <Section id="my-holding" title="Holding" count={nums.holding} hint="Tasks you hold with a live lease.">
          <WorkList
            source={holding}
            label="Held tasks"
            itemKey={(t) => t.id}
            renderItem={(t) => taskRow(t)}
            empty={<Empty title="Nothing in your hands">Serve or claim a task from a board and it shows here while the lease is live.</Empty>}
          />
        </Section>

        <Section id="my-waiting" title="Waiting on me" count={nums.waiting} hint="Questions to you, reviews asked of you, comments to answer.">
          {nobodyWaits ? (
            <Empty title="Nobody is waiting on you">Questions addressed to you and reviews asked of you (or of the team) land here.</Empty>
          ) : (
            <>
              <WaitingList title="Questions to you" icon={CircleHelp} source={questions} itemKey={(x) => x.id} renderItem={(x) => <QuestionRow q={x} />} />
              <WaitingList title="Reviews asked of you" icon={ClipboardCheck} source={reviews} itemKey={(x) => x.id} renderItem={(x) => <ReviewRow r={x} onOpen={open} />} />
              <WaitingList title="Comments to answer" icon={MessageSquare} source={comments} itemKey={(x) => x.id} renderItem={(x) => <CommentRow c={x} onOpen={open} />} />
            </>
          )}
          {poolTotal > 0 && (
            <div>
              <h3 className="mb-1.5 flex items-center gap-2 text-[13px] font-sans font-normal tracking-normal text-muted-foreground">
                <Hourglass className="size-3.5" aria-hidden="true" />
                Open questions nobody picked
                <span className="font-mono text-[11px]">{poolTotal}</span>
              </h3>
              <WorkList source={pool} label="Open questions nobody picked" itemKey={(x) => x.id} renderItem={(x) => <QuestionRow q={x} />} empty={null} />
            </div>
          )}
        </Section>

        <Section id="my-blocked" title="Blocked on me" count={nums.blocked} hint="Work held up by an answer from you or by a task you own.">
          <WorkList
            source={blocked}
            label="Blocked tasks"
            itemKey={(t) => `${t.id}-${t.because}`}
            renderItem={(t) => taskRow(t)}
            empty={
              <Empty title="You are not holding anyone up">
                A task parked on a question you must answer, or one that depends on a task you own that is not done, shows here.
              </Empty>
            }
          />
        </Section>

        <Section id="my-asked" title="Asked by me" count={nums.asked} hint="Questions you asked that are still open.">
          <WorkList
            source={asked}
            more="scroll"
            label="Questions I asked"
            itemKey={(q) => q.id}
            renderItem={(q) => <AskedRow q={q} />}
            empty={<Empty title="No open questions from you">A question you ask a cook or a person stays here until it is answered.</Empty>}
          />
        </Section>
      </div>
    </PageShell>
  );
}
