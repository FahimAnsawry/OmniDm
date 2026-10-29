// SPDX-License-Identifier: GPL-3.0-or-later

// A first, minimal download list. The full screens arrive in milestone M5.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AfterQueue } from "../../protocol/ts/AfterQueue";
import type { DownloadSummary } from "../../protocol/ts/DownloadSummary";
import type { Finished } from "../../protocol/ts/Finished";
import type { Progress } from "../../protocol/ts/Progress";

type Row = DownloadSummary & { speed?: number };

const ACTION_TEXT: Record<string, string> = {
  sleep: "Putting the PC to sleep",
  hibernate: "Hibernating the PC",
  shutdown: "Shutting down the PC",
  quit: "Quitting OmniDm",
};

/** The after-queue countdown, with a way out. */
function AfterQueueBanner({ notice, onClose }: { notice: AfterQueue; onClose: () => void }) {
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
      className="flex items-center gap-4 border-b border-amber-300 bg-amber-50 px-6 py-3 text-amber-950 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
    >
      <p className="flex-1 text-sm font-medium tabular-nums">{message}</p>
      <button
        type="button"
        autoFocus
        className="rounded border border-amber-400 bg-white px-3 py-1 text-sm font-medium hover:bg-amber-100 dark:border-amber-700 dark:bg-amber-900 dark:hover:bg-amber-800"
        onClick={() => (failed ? onClose() : invoke("cancel_after_queue").then(onClose))}
      >
        {failed ? "Dismiss" : "Cancel"}
      </button>
    </div>
  );
}

function formatBytes(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${units[unit]}`;
}

const STATUS_STYLE: Record<string, string> = {
  running: "text-sky-700 dark:text-sky-400",
  completed: "text-emerald-700 dark:text-emerald-400",
  failed: "text-red-700 dark:text-red-400",
  paused: "text-amber-700 dark:text-amber-400",
};

function App() {
  const [rows, setRows] = useState<Row[]>([]);
  const [afterQueue, setAfterQueue] = useState<AfterQueue | null>(null);

  useEffect(() => {
    const refresh = () => invoke<DownloadSummary[]>("list_downloads").then(setRows);
    refresh();
    const unlisten = [
      listen<Progress>("download-progress", ({ payload }) => {
        setRows((current) => {
          if (!current.some((row) => row.downloadId === payload.downloadId)) {
            refresh();
            return current;
          }
          return current.map((row) =>
            row.downloadId === payload.downloadId
              ? {
                  ...row,
                  status: "running",
                  downloaded: payload.downloaded,
                  total: payload.total ?? row.total,
                  speed: payload.speed,
                }
              : row,
          );
        });
      }),
      listen<Finished>("download-finished", () => refresh()),
      // Shown while counting down, and when it failed; gone once done or cancelled.
      listen<AfterQueue>("after-queue", ({ payload }) =>
        setAfterQueue(payload.outcome == null || payload.outcome === "failed" ? payload : null),
      ),
    ];
    return () => {
      unlisten.forEach((promise) => promise.then((stop) => stop()));
    };
  }, []);

  const act = (command: "pause_download" | "resume_download", id: number) =>
    invoke(command, { id }).catch((error) => console.error(error));

  return (
    <main className="min-h-screen bg-neutral-100 font-sans text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      <header className="border-b border-neutral-200 px-6 py-4 dark:border-neutral-800">
        <h1 className="text-xl font-semibold">OmniDm</h1>
      </header>
      {afterQueue && (
        <AfterQueueBanner notice={afterQueue} onClose={() => setAfterQueue(null)} />
      )}
      {rows.length === 0 ? (
        <p className="px-6 py-10 text-neutral-500">
          No downloads yet. Click a download link in your browser and it appears here.
        </p>
      ) : (
        <ul className="divide-y divide-neutral-200 dark:divide-neutral-800">
          {rows.map((row) => {
            const percent = row.total ? Math.min(100, (row.downloaded / row.total) * 100) : 0;
            const active = row.status === "running" || row.status === "queued";
            return (
              <li key={row.downloadId} className="flex items-center gap-4 px-6 py-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium" title={row.url}>
                    {row.fileName ?? row.url}
                  </p>
                  <div className="mt-1.5 h-1.5 overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800">
                    <div
                      className="h-full bg-sky-600 transition-[width]"
                      style={{ width: `${row.status === "completed" ? 100 : percent}%` }}
                    />
                  </div>
                  <p className="mt-1 text-xs text-neutral-500">
                    {formatBytes(row.downloaded)}
                    {row.total != null && ` of ${formatBytes(row.total)}`}
                    {row.status === "running" && row.speed != null && ` · ${formatBytes(row.speed)}/s`}
                    {row.error && ` · ${row.error}`}
                  </p>
                </div>
                <span className={`w-24 text-sm capitalize ${STATUS_STYLE[row.status] ?? ""}`}>
                  {row.status}
                </span>
                <button
                  type="button"
                  className="w-20 rounded border border-neutral-300 px-2 py-1 text-sm hover:bg-neutral-200 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
                  disabled={row.status === "completed"}
                  onClick={() => act(active ? "pause_download" : "resume_download", row.downloadId)}
                >
                  {active ? "Pause" : "Resume"}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </main>
  );
}

export default App;
