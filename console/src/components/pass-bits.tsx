import { Cloud, Hash } from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar } from "./bits";
import type { CookInfo } from "./pass-data";

/** A cook's presence on an avatar: saffron while awake (pulsing while it cooks), grey when idle, a cloud for cloud cooks. */
export function PresenceMark({ cook, ring = "ring-card", className }: { cook: CookInfo; ring?: string; className?: string }) {
  const { state } = cook;
  if (cook.cloud) {
    return (
      <span
        aria-hidden="true"
        className={cn("absolute -right-1 -bottom-1 grid size-4 place-items-center rounded-full bg-card text-muted-foreground ring-2", ring, className)}
      >
        <Cloud className={cn("size-2.5", (state === "awake" || state === "cooking") && "text-corn-text")} />
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      className={cn(
        "absolute -right-0.5 -bottom-0.5 size-3 rounded-full ring-2",
        ring,
        state === "cooking" && "live-dot bg-corn",
        state === "awake" && "bg-corn",
        state === "idle" && "bg-muted-foreground/40",
        state === "disabled" && "bg-destructive",
        className,
      )}
    />
  );
}

/** A chef's avatar (a person, or a cook with its presence), or the channel's. */
export function ConvAvatar({
  name,
  cook,
  channel,
  size = "size-10",
  ring,
  className,
}: {
  name: string;
  cook?: CookInfo | undefined;
  channel?: boolean;
  size?: string;
  ring?: string;
  className?: string;
}) {
  return (
    <span className={cn("relative inline-flex shrink-0", className)}>
      {channel ? (
        <span className={cn("grid place-items-center rounded-xl bg-corn text-corn-foreground", size)}>
          <Hash className="size-[45%]" aria-hidden="true" />
        </span>
      ) : (
        <Avatar name={name} className={cn("rounded-xl text-[15px]", size)} />
      )}
      {cook && <PresenceMark cook={cook} {...(ring ? { ring } : {})} />}
    </span>
  );
}

/** What a cook is, as a small quiet chip: "Claude Code". */
export function KindChip({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span className={cn("inline-flex shrink-0 items-center rounded-md bg-muted px-1.5 py-px text-[10.5px] leading-4 font-medium text-muted-foreground", className)}>
      {children}
    </span>
  );
}

export function stateText(cook: CookInfo): string {
  return cook.state === "cooking" ? "cooking" : cook.state;
}
