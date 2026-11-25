import { useLayoutEffect, type RefObject } from "react";

/**
 * Sets `--fill-top` (px) on the element: where it starts in the document. A pane that fills the viewport below the
 * page header sizes itself from it in CSS (`h-[calc(100dvh-var(--fill-top,10rem)-1rem)]`), so the page itself need not
 * scroll past it. Re-measured when the window, the page or the element's parent changes size (a header that wraps,
 * a filter row that opens). A custom property on a ref instead of a `style` prop or state.
 */
export function useFillTop(ref: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const calc = () => el.style.setProperty("--fill-top", `${Math.round(el.getBoundingClientRect().top + window.scrollY)}px`);
    calc();
    window.addEventListener("resize", calc);
    const ro = new ResizeObserver(calc);
    ro.observe(document.body);
    if (el.parentElement) ro.observe(el.parentElement);
    return () => {
      window.removeEventListener("resize", calc);
      ro.disconnect();
    };
  }, [ref]);
}
