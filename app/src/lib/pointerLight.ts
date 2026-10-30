// SPDX-License-Identifier: GPL-3.0-or-later

/** The buttons whose light follows the mouse (index.css). */
const LIT = ".button-primary, .button-glass";

/** Tells the button under the mouse where the mouse is, as --mx/--my, for its light. */
export function followPointer() {
  document.addEventListener(
    "pointermove",
    (event) => {
      const button = event.target instanceof Element ? event.target.closest<HTMLElement>(LIT) : null;
      if (!button) return;
      const box = button.getBoundingClientRect();
      button.style.setProperty("--mx", `${event.clientX - box.left}px`);
      button.style.setProperty("--my", `${event.clientY - box.top}px`);
    },
    { passive: true },
  );
}
