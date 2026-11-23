import { useSyncExternalStore } from "react";

/**
 * Layout tiers (docs/design.md, "Mobile and tablet"): phone below 768px (bottom tabs), tablet 768 to
 * 1023px (icon rail), desktop from 1024px (full sidebar). CSS uses the matching Tailwind
 * breakpoints (`md:`, `lg:`); these hooks are for the few places that need it in JS.
 */
export const PHONE_QUERY = "(max-width: 767px)";
export const DESKTOP_QUERY = "(min-width: 1024px)";

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const m = window.matchMedia(query);
      m.addEventListener("change", onChange);
      return () => m.removeEventListener("change", onChange);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

export const useIsPhone = () => useMediaQuery(PHONE_QUERY);
export const useIsDesktop = () => useMediaQuery(DESKTOP_QUERY);
