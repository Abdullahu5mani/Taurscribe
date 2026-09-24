import { useEffect } from "react";

/** Width buckets; keep in sync with the compact/wide rules in the CSS. */
const COMPACT_MAX = 760;
const WIDE_MIN = 1200;

export type WindowLayout = "compact" | "regular" | "wide";

function layoutFor(width: number): WindowLayout {
  return width <= COMPACT_MAX ? "compact" : width >= WIDE_MIN ? "wide" : "regular";
}

/**
 * Makes resizing feel smooth, via attributes on <html>:
 *  - data-resizing: set while the window is being resized. CSS pauses
 *    transitions meanwhile, so sizes follow the window edge instead of
 *    easing behind it (which reads as lag).
 *  - data-layout: compact / regular / wide.
 *  - data-layout-changed: set briefly when the bucket changes, so the areas
 *    that re-flow can play a short settle animation instead of jumping.
 */
export function useWindowLayout() {
  useEffect(() => {
    const root = document.documentElement;
    let layout = layoutFor(window.innerWidth);
    root.dataset.layout = layout;

    let frame = 0;
    let idleTimer: ReturnType<typeof setTimeout> | undefined;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;

    const onResize = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        root.dataset.resizing = "";
        clearTimeout(idleTimer);
        idleTimer = setTimeout(() => {
          delete root.dataset.resizing;
        }, 160);

        const next = layoutFor(window.innerWidth);
        if (next !== layout) {
          layout = next;
          root.dataset.layout = next;
          root.dataset.layoutChanged = "";
          clearTimeout(settleTimer);
          settleTimer = setTimeout(() => {
            delete root.dataset.layoutChanged;
          }, 380);
        }
      });
    };

    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      cancelAnimationFrame(frame);
      clearTimeout(idleTimer);
      clearTimeout(settleTimer);
      delete root.dataset.resizing;
      delete root.dataset.layoutChanged;
    };
  }, []);
}
