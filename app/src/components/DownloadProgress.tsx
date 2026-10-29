// SPDX-License-Identifier: GPL-3.0-or-later

// One download's progress, shown in the capture prompt's window after "Download now".
// Closing the window leaves the download running.

import { type ReactNode, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Finished } from "../../../protocol/ts/Finished";
import type { Progress } from "../../../protocol/ts/Progress";
import { api, errorText } from "../lib/api";
import { formatBytes, formatDuration, formatSpeed } from "../lib/format";
import { Button } from "./Fields";
import { Icon } from "./Icon";

type State = "starting" | "running" | "paused" | "completed" | "failed";

export function DownloadProgress({ id, name, size }: { id: number; name: string; size?: number }) {
  const [state, setState] = useState<State>("starting");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(size);
  const [speed, setSpeed] = useState(0);
  const [connections, setConnections] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const stops = [
      listen<Progress[]>("download-progress", ({ payload }) => {
        const mine = payload.find((p) => p.downloadId === id);
        if (!mine) return;
        setState("running");
        setDownloaded(mine.downloaded);
        if (mine.total != null) setTotal(mine.total);
        setSpeed(mine.speed);
        setConnections(mine.connections);
      }),
      listen<Finished>("download-finished", ({ payload }) => {
        if (payload.downloadId !== id) return;
        setSpeed(0);
        setError(payload.error ?? null);
        setState(payload.status);
      }),
    ];
    // A small file may have finished before this view was listening.
    Promise.all(stops)
      .then(() => api.listDownloads())
      .then((list) => {
        const now = list.find((d) => d.downloadId === id);
        if (!now) return;
        if (now.total != null) setTotal(now.total);
        setDownloaded((d) => Math.max(d, now.downloaded));
        if (now.status === "completed" || now.status === "failed" || now.status === "paused") {
          setState(now.status);
          setError(now.error ?? null);
        }
      })
      .catch(() => {});
    return () => stops.forEach((stop) => stop.then((f) => f()));
  }, [id]);

  const fraction = state === "completed" ? 1 : total ? Math.min(1, downloaded / total) : 0;
  const percent = Math.floor(fraction * 100);

  useEffect(() => {
    const window = getCurrentWindow();
    const title =
      state === "completed" ? `Done · ${name}` : state === "failed" ? `Failed · ${name}` : `${percent}% · ${name}`;
    window.setTitle(title).catch(() => {});
  }, [state, percent, name]);

  const act = (action: Promise<unknown>) => action.catch((e) => setError(errorText(e)));
  const close = () => getCurrentWindow().close();
  const left = state === "running" && speed > 0 && total != null ? (total - downloaded) / speed : NaN;

  const headline: Record<State, string> = {
    starting: "Starting…",
    running: "Downloading",
    paused: "Paused",
    completed: "Download complete",
    failed: "Download failed",
  };
  const bar =
    state === "completed"
      ? "bg-emerald-500"
      : state === "failed"
        ? "bg-red-500"
        : state === "paused"
          ? "bg-neutral-400 dark:bg-neutral-500"
          : "bg-sky-500";

  return (
    <div className="flex h-screen flex-col gap-4 bg-white p-5 font-sans text-neutral-900 select-none dark:bg-neutral-900 dark:text-neutral-100">
      <div className="flex items-start gap-3">
        <span
          className={`rounded-lg p-2 ${
            state === "completed"
              ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/50 dark:text-emerald-300"
              : state === "failed"
                ? "bg-red-100 text-red-700 dark:bg-red-900/50 dark:text-red-300"
                : "bg-sky-100 text-sky-700 dark:bg-sky-900/50 dark:text-sky-300"
          }`}
        >
          <Icon
            name={state === "completed" ? "completed" : state === "failed" ? "failed" : "downloading"}
            className="size-5"
          />
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="text-[15px] font-semibold" aria-live="polite">
            {headline[state]}
          </h1>
          <p className="truncate text-[13px] text-neutral-600 dark:text-neutral-400" title={name}>
            {name}
          </p>
        </div>
        <span className="text-2xl font-semibold tabular-nums">{percent}%</span>
      </div>

      <div
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-label="Progress"
        className="h-2.5 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800"
      >
        <div className={`h-full rounded-full transition-[width] duration-200 ${bar}`} style={{ width: `${fraction * 100}%` }} />
      </div>

      <dl className="grid grid-cols-3 gap-3 text-[13px] tabular-nums">
        <Stat label="Downloaded">
          {formatBytes(state === "completed" && total != null ? total : downloaded)}
          {total != null && <span className="text-neutral-500"> of {formatBytes(total)}</span>}
        </Stat>
        <Stat label="Speed">
          {state === "running" ? formatSpeed(speed) : "—"}
          {state === "running" && connections > 0 && (
            <span className="text-neutral-500">
              {" "}
              · {connections} {connections === 1 ? "connection" : "connections"}
            </span>
          )}
        </Stat>
        <Stat label="Time left">{Number.isFinite(left) ? formatDuration(left) : "—"}</Stat>
      </dl>

      {error && (
        <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center justify-end gap-2">
        {state === "completed" ? (
          <>
            <Button onClick={() => act(api.showInFolder(id))} className="mr-auto">
              <Icon name="folder" className="size-4 opacity-70" />
              Show in folder
            </Button>
            <Button onClick={close}>Close</Button>
            <Button primary autoFocus onClick={() => act(api.openFile(id)).then(close)}>
              <Icon name="open" className="size-4" />
              Open
            </Button>
          </>
        ) : (
          <>
            <span className="mr-auto text-xs text-neutral-500">Closing this window doesn't stop the download.</span>
            {state === "paused" || state === "failed" ? (
              <Button
                onClick={() => {
                  setError(null);
                  setState("starting");
                  act(api.resume([id]));
                }}
              >
                <Icon name="play" className="size-4" />
                {state === "failed" ? "Retry" : "Resume"}
              </Button>
            ) : (
              <Button onClick={() => act(api.pause([id]))}>
                <Icon name="pause" className="size-4" />
                Pause
              </Button>
            )}
            <Button primary autoFocus onClick={close}>
              Close
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-xs text-neutral-500">{label}</dt>
      <dd className="mt-0.5 font-medium">{children}</dd>
    </div>
  );
}
