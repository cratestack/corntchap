import type { UsageReport } from "@corntchap/api";
import { cn } from "@/lib/utils";

/** Estimated tokens, short: 850, 12k, 1.2M. */
export function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, "")}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

/** A tiny per-day bar row (plain divs): the estimated tokens corntchap sent each day, oldest first. */
export function DayBars({ days, className }: { days: UsageReport["by_day"]; className?: string }) {
  const max = Math.max(1, ...days.map((d) => d.tokens));
  return (
    <div className={cn("flex h-8 items-end gap-1", className)} role="img" aria-label="Estimated tokens sent per day">
      {days.map((d) => (
        <div
          key={d.day}
          title={`${d.day}: ~${d.tokens.toLocaleString()} tokens in ${d.calls.toLocaleString()} sends`}
          className="flex h-full min-w-0 flex-1 items-end"
        >
          <div
            className={cn("w-full rounded-[2px]", d.tokens > 0 ? "bg-corn" : "bg-muted")}
            style={{ height: d.tokens > 0 ? `${Math.max(8, Math.round((d.tokens / max) * 100))}%` : "4px" }}
          />
        </div>
      ))}
    </div>
  );
}

/** What corntchap sent one cook over the last days: an estimate, tokens ~ characters / 4. */
export function CookUsage({ report }: { report: UsageReport | undefined }) {
  if (!report) return <p className="pt-2 text-[12.5px] text-muted-foreground">Counting...</p>;
  const top = report.top.slice(0, 3);
  return (
    <div className="pt-2">
      <p className="text-[12.5px]">
        <span className="font-mono font-bold">~{fmtTokens(report.totals.tokens)}</span> tokens sent in {report.days} days
        <span className="text-muted-foreground"> · {report.totals.calls.toLocaleString()} sends</span>
      </p>
      <DayBars days={report.by_day} className="mt-2" />
      {top.length > 0 && (
        <ul className="mt-2 space-y-0.5 text-[12px] text-muted-foreground">
          {top.map((t) => (
            <li key={`${t.kind}:${t.name}`} className="flex justify-between gap-3">
              <span className="min-w-0 truncate font-mono">{t.name}</span>
              <span className="shrink-0">~{fmtTokens(t.tokens)}</span>
            </li>
          ))}
        </ul>
      )}
      <p className="mt-2 text-[11.5px] text-muted-foreground">{report.note}</p>
    </div>
  );
}
