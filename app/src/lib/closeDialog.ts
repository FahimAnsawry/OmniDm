// SPDX-License-Identifier: GPL-3.0-or-later

/** How long a dialog takes to leave (`dialog.leaving` in index.css), in ms. */
const LEAVE_MS = 140;

/** Plays the dialog's leaving, then calls `then` (which takes it off the page). */
export function closeDialog(dialog: HTMLDialogElement | null, then: () => void) {
  if (!dialog) return then();
  if (dialog.classList.contains("leaving")) return;
  dialog.classList.add("leaving");
  setTimeout(then, LEAVE_MS);
}
