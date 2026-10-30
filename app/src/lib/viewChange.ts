// SPDX-License-Identifier: GPL-3.0-or-later

const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
/** Only the first rows come in one after another; the rest are there at once. */
const STAGGERED_ROWS = 12;

/**
 * Plays a panel changing to another list or page: its parts marked `data-view-in`
 * (the title, an empty list's picture) rise in, then its first rows, one after another.
 * Rows keep their place in `transform`, so they rise with `translate`.
 */
export function playViewChange(panel: HTMLElement | null) {
  if (!panel) return;
  const less = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const from = less ? { opacity: 0 } : { opacity: 0, translate: "0 8px" };
  const to = less ? { opacity: 1 } : { opacity: 1, translate: "0 0" };
  for (const part of panel.querySelectorAll<HTMLElement>("[data-view-in]")) {
    part.animate([from, to], { duration: 240, easing: EASE, fill: "backwards" });
  }
  const rows = [...panel.querySelectorAll<HTMLElement>('[role="row"][aria-rowindex]:not([aria-rowindex="1"])')]
    .sort((a, b) => Number(a.getAttribute("aria-rowindex")) - Number(b.getAttribute("aria-rowindex")))
    .slice(0, STAGGERED_ROWS);
  rows.forEach((row, i) =>
    row.animate([from, to], { duration: 260, delay: 40 + i * 20, easing: EASE, fill: "backwards" }),
  );
}
