// SPDX-License-Identifier: GPL-3.0-or-later

import type { AfterQueue } from "../../../protocol/ts/AfterQueue";
import { api } from "../lib/api";

const ACTION_TEXT: Record<string, string> = {
  sleep: "Putting the PC to sleep",
  hibernate: "Hibernating the PC",
  shutdown: "Shutting down the PC",
  quit: "Quitting OmniDm",
};

/** The after-queue countdown, with a way out. */
export function AfterQueueBanner({ notice, onClose }: { notice: AfterQueue; onClose: () => void }) {
  const doing = ACTION_TEXT[notice.action] ?? notice.action;
  const failed = notice.outcome === "failed";
  const message = failed
    ? `${doing} didn't work. ${notice.detail ?? ""}`
    : notice.waitingForDownloads
      ? `${notice.queue} finished. ${doing} once the other downloads end.`
      : `${notice.queue} finished. ${doing} in ${notice.secondsLeft} s.`;
  return (
    <div
      role="alert"
      className="flex items-center gap-4 border-b border-amber-300 bg-amber-50 px-4 py-2.5 text-amber-950 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
    >
      <p className="flex-1 text-[13px] font-medium tabular-nums">{message}</p>
      <button
        type="button"
        autoFocus
        className="rounded-md border border-amber-400 bg-white px-3 py-1 text-[13px] font-medium hover:bg-amber-100 dark:border-amber-700 dark:bg-amber-900 dark:hover:bg-amber-800"
        onClick={() => (failed ? onClose() : api.cancelAfterQueue().then(onClose))}
      >
        {failed ? "Dismiss" : "Cancel"}
      </button>
    </div>
  );
}
