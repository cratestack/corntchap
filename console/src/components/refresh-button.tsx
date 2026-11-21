import { useEffect, useRef, useState } from "react";
import { useIsFetching } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useInvalidateAll } from "@/lib/data";
import { cn } from "@/lib/utils";

/**
 * One refresh control for every page and sheet: refetches the Refine queries that are on screen (everything lives
 * under the ["data"] key). The icon spins while anything is in flight, and for at least a beat so a fast refetch
 * still reads as having happened. The `r` shortcut lives in AppShell.
 */
export function RefreshButton({ className, size = "icon" }: { className?: string; size?: "icon" | "icon-sm" }) {
  const invalidate = useInvalidateAll();
  const fetching = useIsFetching({ queryKey: ["data"] }) > 0;
  const [pressed, setPressed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const spinning = fetching || pressed;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size={size}
          aria-label="Refresh"
          aria-busy={spinning}
          className={cn("shrink-0", className)}
          onClick={() => {
            setPressed(true);
            clearTimeout(timer.current);
            timer.current = setTimeout(() => setPressed(false), 700);
            void invalidate();
          }}
        >
          <RefreshCw className={cn("size-4", spinning && "animate-spin")} aria-hidden="true" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Refresh (R)</TooltipContent>
    </Tooltip>
  );
}
