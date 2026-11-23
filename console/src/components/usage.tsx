import type { UsageReport } from "@corntchap/api";
import { cn } from "@/lib/utils";

/** Estimated tokens, short: 850, 12k, 1.2M. */
export function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, "")}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

const DAY_LETTERS = ["S", "M", "T", "W", "T", "F", "S"];
const dayLetter = (day: string) => DAY_LETTERS[new Date(`${day}T12:00:00`).getDay()] ?? "";

/** A tiny per-day bar row (plain divs): the estimated tokens corntchap sent each day, oldest first, with weekday initials. */
export function DayBars({ days, className }: { days: UsageReport["by_day"]; className?: string }) {
  const max = Math.max(1, ...days.map((d) => d.tokens));
  return (
    <div className={cn("flex items-end gap-1.5", className)} role="img" aria-label="Estimated tokens sent per day">
      {days.map((d) => (
        <div
          key={d.day}
          title={`${d.day}: ~${d.tokens.toLocaleString()} tokens in ${d.calls.toLocaleString()} sends`}
          className="flex min-w-0 flex-1 flex-col items-center gap-1"
        >
          <div className="flex h-10 w-full items-end">
            <div
              className={cn("w-full rounded-[3px] transition-[height] duration-300 motion-reduce:transition-none", d.tokens > 0 ? "bg-corn" : "bg-muted")}
              style={{ height: d.tokens > 0 ? `${Math.max(10, Math.round((d.tokens / max) * 100))}%` : "3px" }}
            />
          </div>
          {days.length <= 14 && <span className="font-mono text-[10px] leading-none text-muted-foreground">{dayLetter(d.day)}</span>}
        </div>
      ))}
    </div>
  );
}

/** One labelled line with a proportional bar and the estimate on the right. */
function BarRow({ label, caption, tokens, max }: { label: string; caption?: string; tokens: number; max: number }) {
  return (
    <li className="grid grid-cols-[minmax(0,7.5rem)_minmax(0,1fr)_3.25rem] items-center gap-3 text-[12.5px] sm:grid-cols-[minmax(0,9rem)_minmax(0,1fr)_3.5rem]">
      <span className="min-w-0 truncate" title={caption ? `${label} (${caption})` : label}>
        <span className="font-medium">{label}</span>
        {caption && <span className="ml-1.5 font-mono text-[10.5px] text-muted-foreground">{caption}</span>}
      </span>
      <span className="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
        <span className="block h-full rounded-full bg-corn" style={{ width: `${Math.max(3, Math.round((tokens / Math.max(1, max)) * 100))}%` }} />
      </span>
      <span className="text-right font-mono text-[12px] tabular-nums text-muted-foreground">~{fmtTokens(tokens)}</span>
    </li>
  );
}

/** Estimated tokens corntchap sent (a pot's or one cook's): the total, a bar per day, then who and what took most. tokens ~ characters / 4. */
export function UsagePanel({ report, showCooks }: { report: UsageReport | undefined; showCooks?: boolean }) {
  if (!report) return <p className="text-[13px] text-muted-foreground">Counting...</p>;
  const cooks = showCooks ? report.by_cook.slice(0, 5) : [];
  const top = report.top.slice(0, 3);
  const cookMax = Math.max(1, ...cooks.map((c) => c.tokens));
  const topMax = Math.max(1, ...top.map((t) => t.tokens));
  return (
    <div className="space-y-4">
      <div className="flex items-baseline justify-between gap-3">
        <p>
          <span className="font-mono text-[22px] font-bold leading-none tabular-nums">~{fmtTokens(report.totals.tokens)}</span>
          <span className="ml-1.5 text-[13px] text-muted-foreground">tokens in {report.days} days</span>
        </p>
        <p className="font-mono text-[12px] text-muted-foreground">{report.totals.calls.toLocaleString()} sends</p>
      </div>
      <DayBars days={report.by_day} />
      {cooks.length > 0 && (
        <div>
          <p className="label-caps mb-2">By cook</p>
          <ul className="space-y-2">
            {cooks.map((c) => (
              <BarRow key={c.cook} label={c.cook} tokens={c.tokens} max={cookMax} />
            ))}
          </ul>
        </div>
      )}
      {top.length > 0 && (
        <div>
          <p className="label-caps mb-2">Most sent</p>
          <ul className="space-y-2">
            {top.map((t) => (
              <BarRow key={`${t.kind}:${t.name}`} label={t.name} caption={t.kind} tokens={t.tokens} max={topMax} />
            ))}
          </ul>
        </div>
      )}
      <p className="text-[11.5px] text-muted-foreground">{report.note}</p>
    </div>
  );
}
