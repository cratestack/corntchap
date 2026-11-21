import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { PaletteButton } from "./shell";

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
    <div className={cn("px-5 pt-6 pb-16 lg:px-8", className)}>
      <header className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0">
          {eyebrow && <p className="label-caps">{eyebrow}</p>}
          <h1 className="mt-1 text-[30px] leading-[1.1] sm:text-[34px]">{title}</h1>
          {sub && <p className="mt-1.5 max-w-2xl text-[13.5px] text-muted-foreground">{sub}</p>}
        </div>
        <div className="flex items-center gap-2">
          <PaletteButton />
          {actions}
        </div>
      </header>
      <div className="mt-6">{children}</div>
    </div>
  );
}

/** A restrained table surface for the admin pages. */
export function DataTable({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("overflow-x-auto rounded-[14px] border border-border bg-card", className)}>
      <table className="w-full min-w-[560px] border-collapse text-left text-[13.5px]">{children}</table>
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
