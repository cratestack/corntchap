import type { ReactNode } from "react";
import { Link } from "react-router";
import { ChevronLeft } from "lucide-react";
import { useQueries } from "@tanstack/react-query";
import { cn } from "@/lib/utils";
import { v1 } from "@/lib/api";
import type { WakeTargetRow } from "@/lib/data";

/** "Back to Cooks": the breadcrumb on top of every details page. */
export function Crumb({ to, parent, here }: { to: string; parent: string; here: string }) {
  return (
    <nav aria-label="Breadcrumb" className="px-3 pt-2 sm:px-5 sm:pt-5 lg:px-8">
      <ol className="flex flex-wrap items-center gap-1.5 text-[13px] text-muted-foreground sm:text-[12.5px]">
        <li>
          <Link to={to} className="inline-flex h-10 items-center gap-1 rounded-md pr-2 pl-1 hover:text-foreground focus-visible:text-foreground sm:h-auto sm:pl-0">
            <ChevronLeft className="size-3.5" aria-hidden="true" /> Back to {parent}
          </Link>
        </li>
        <li aria-hidden="true" className="max-sm:hidden">/</li>
        <li aria-current="page" className="truncate font-mono text-foreground max-sm:sr-only">
          {here}
        </li>
      </ol>
    </nav>
  );
}

/** A titled block of a details page: a real h2, an optional hint and aside, then the content. */
export function Section({
  title,
  hint,
  aside,
  children,
  className,
}: {
  title: string;
  hint?: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section aria-label={title} className={cn("min-w-0 rounded-[14px] border border-border bg-card p-4", className)}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h2 className="text-[17px] leading-tight">{title}</h2>
        {aside}
      </div>
      {hint && <p className="mt-1 text-[12.5px] text-muted-foreground">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

/** What a section shows when there is nothing to show. */
export function Quiet({ children }: { children: ReactNode }) {
  return <p className="text-[12.5px] text-muted-foreground">{children}</p>;
}

/** Definition rows for the headers. */
export function Facts({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 text-[13px] sm:grid-cols-[auto_minmax(0,1fr)_auto_minmax(0,1fr)]">{children}</dl>;
}
export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </>
  );
}

export const linkClass = "underline-offset-2 hover:underline focus-visible:underline";

/** A pot's key as a link to its details page. */
export function PotLink({ pot, children }: { pot: string; children?: ReactNode }) {
  return (
    <Link to={`/pots/${pot}/about`} className={cn("font-mono text-[12px] font-bold", linkClass)}>
      {children ?? pot}
    </Link>
  );
}

/** The wake targets of several cooks at once (same query keys as `useV1`, so the cooks page and this share the cache). */
export function useWakeTargetsOf(names: string[]) {
  const results = useQueries({
    queries: names.map((n) => ({
      queryKey: ["data", "v1", `/agents/${n}/wake-targets`],
      queryFn: () => v1<WakeTargetRow[]>("GET", `/agents/${n}/wake-targets`),
      staleTime: 1500,
    })),
  });
  const byCook = new Map<string, WakeTargetRow[]>();
  names.forEach((n, i) => byCook.set(n, results[i]?.data ?? []));
  return { byCook, isLoading: results.some((r) => r.isLoading) };
}
