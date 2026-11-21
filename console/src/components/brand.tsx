import { useId } from "react";
import { cn } from "@/lib/utils";

/** The cooking pot: terracotta body, cream rim line, three saffron steam strokes (24px grid). */
export function LogoMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} role="img" aria-label="corntchap">
      <path d="M4.5 12.5h15v2.4a5.6 5.6 0 0 1-5.6 5.6h-3.8a5.6 5.6 0 0 1-5.6-5.6z" fill="#c8502c" />
      <path d="M3.2 12.5h17.6" stroke="#fbf5e9" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M2.6 13.6h1.9M19.5 13.6h1.9" stroke="#c8502c" strokeWidth="1.6" strokeLinecap="round" />
      <g fill="none" stroke="#f2b33d" strokeWidth="1.5" strokeLinecap="round">
        <path d="M8.6 10.1c-1.1-1.1 1.1-1.8 0-3" />
        <path d="M12 10.1c-1.1-1.1 1.1-1.8 0-3" />
        <path d="M15.4 10.1c-1.1-1.1 1.1-1.8 0-3" />
      </g>
    </svg>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={cn("font-display text-[19px] leading-none tracking-tight", className)}>corntchap</span>
  );
}

/**
 * The wax-print band (concentric circles + zigzag). Decorative only: sign-in, device approval and empty
 * states, never a dense working surface.
 */
export function WaxBand({ className, height = 64 }: { className?: string; height?: number }) {
  const id = useId().replace(/:/g, "");
  return (
    <svg aria-hidden="true" focusable="false" className={cn("block w-full", className)} height={height} preserveAspectRatio="xMinYMid slice">
      <defs>
        <pattern id={`c${id}`} width="44" height="44" patternUnits="userSpaceOnUse">
          <circle cx="22" cy="22" r="20" fill="#c8502c" />
          <circle cx="22" cy="22" r="15" fill="#fbf5e9" />
          <circle cx="22" cy="22" r="10.5" fill="#1f7a72" />
          <circle cx="22" cy="22" r="6" fill="#f2b33d" />
          <circle cx="22" cy="22" r="2.2" fill="#15183a" />
        </pattern>
        <pattern id={`z${id}`} width="22" height="12" patternUnits="userSpaceOnUse">
          <path d="M0 9 L5.5 3 L11 9 L16.5 3 L22 9" fill="none" stroke="#f2b33d" strokeWidth="2" strokeLinejoin="round" />
        </pattern>
      </defs>
      <rect width="100%" height={Math.round(height * 0.7)} fill={`url(#c${id})`} />
      <rect y={Math.round(height * 0.74)} width="100%" height="12" fill={`url(#z${id})`} />
    </svg>
  );
}
