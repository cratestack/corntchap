import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { PaletteButton } from "./shell";
import { RefreshButton } from "./refresh-button";

export function PageShell({
  title,
  eyebrow,
  sub,
  actions,
  children,
  className,
}: {
  title: string;
  eyebrow?: string;
  sub?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("px-4 pt-4 pb-10 md:px-5 md:pt-6 md:pb-16 lg:px-8", className)}>
      <header>
        <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
          <div className="min-w-0 flex-1 basis-40">
            {eyebrow && <p className="label-caps">{eyebrow}</p>}
            <h1 className="mt-1 text-[26px] leading-[1.1] sm:text-[34px]">{title}</h1>
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            <PaletteButton />
            <RefreshButton />
            {actions}
          </div>
        </div>
        {sub && <div className="mt-2 max-w-2xl text-[13.5px] text-muted-foreground">{sub}</div>}
      </header>
      <div className="mt-6">{children}</div>
    </div>
  );
}

/** A restrained table surface for the admin pages. */
export function DataTable({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("overflow-clip rounded-[14px] border border-border bg-card", className)}>
      <table className="w-full border-collapse text-left text-[13.5px]">{children}</table>
    </div>
  );
}

export const TH = ({ children, className }: { children?: ReactNode; className?: string }) => (
  <th scope="col" className={cn("label-caps border-b border-border px-4 py-2.5 font-normal", className)}>
    {children}
  </th>
);

export const TD = ({ children, className }: { children?: ReactNode; className?: string }) => (
  <td className={cn("border-b border-border/70 px-4 py-3 align-middle last:border-b-0", className)}>{children}</td>
);
