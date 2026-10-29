// SPDX-License-Identifier: GPL-3.0-or-later

// One download's progress, shown in the capture prompt's window after "Download now".
// Closing the window while downloading asks for confirmation and cancels the download.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Finished } from "../../../protocol/ts/Finished";
import type { Progress } from "../../../protocol/ts/Progress";
import { api, errorText } from "../lib/api";
import { formatBytes, formatElapsed, formatSpeed } from "../lib/format";
import { Chip, Crumbs, EnterHint, FileIcon, SmallTitleBar, folderCrumbs } from "./Chrome";
import { Icon } from "./Icon";
import { SpeedGraph } from "./SpeedGraph";

type State = "starting" | "running" | "paused" | "failed" | "completed";

/** Speed samples kept for the graph, one a second. */
const SAMPLES = 40;

type Props = {
  id: number;
  name: string;
  size?: number;
  folder: string;
  downloadsFolder?: string;
  /** A name whose extension picks the file icon, when `name` has none. */
  iconName?: string;
  /** False in a window that has the system's title bar. */
  titleBar?: boolean;
};

export function DownloadProgress({ id, name, size, folder, downloadsFolder, iconName = name, titleBar = true }: Props) {
  const [state, setState] = useState<State>("starting");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(size);
  const [speed, setSpeed] = useState(0);
  const [samples, setSamples] = useState<number[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  /** When it started, and how long it took once done. */
  const startedAt = useRef(Date.now());
  const [took, setTook] = useState<number | null>(null);
  const lastSample = useRef(0);
  const allowClose = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    const stops = [
      listen<Progress[]>("download-progress", ({ payload }) => {
        const mine = payload.find((p) => p.downloadId === id);
        if (!mine) return;
        setState("running");
        setDownloaded(mine.downloaded);
        if (mine.total != null) setTotal(mine.total);
        setSpeed(mine.speed);
        const now = Date.now();
        if (now - lastSample.current >= 1000) {
          lastSample.current = now;
          setSamples((list) => [...list, mine.speed].slice(-SAMPLES));
        }
      }),
      listen<Finished>("download-finished", ({ payload }) => {
        if (payload.downloadId !== id) return;
        setSpeed(0);
        setError(payload.error ?? null);
        setState(payload.status);
        if (payload.status === "completed") {
          setConfirmCancel(false);
          setTook((Date.now() - startedAt.current) / 1000);
        }
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
          if (now.status === "completed") setTook((Date.now() - startedAt.current) / 1000);
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

  // Intercept OS window close (e.g. Alt+F4 or taskbar close) while the download is not completed.
  useEffect(() => {
    const stop = getCurrentWindow().onCloseRequested((event) => {
      if (!allowClose.current && stateRef.current !== "completed") {
        event.preventDefault();
        setConfirmCancel(true);
      }
    });
    return () => void stop.then((f) => f());
  }, []);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (confirmCancel) {
          if (!cancelling) setConfirmCancel(false);
        } else if (state === "completed") {
          closeNow();
        } else {
          setConfirmCancel(true);
        }
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [confirmCancel, cancelling, state]);

  const act = (action: Promise<unknown>) => action.catch((e) => setError(errorText(e)));
  const closeNow = () => {
    allowClose.current = true;
    const win = getCurrentWindow();
    void win.destroy().catch(() => win.close());
  };
  const requestClose = () => {
    if (state === "completed") closeNow();
    else setConfirmCancel(true);
  };
  const cancelAndClose = async () => {
    setCancelling(true);
    try {
      await api.remove([id], true);
    } catch {
      // Even if already removed, close the window.
    } finally {
      closeNow();
    }
  };
  const left = state === "running" && speed > 0 && total != null ? (total - downloaded) / speed : NaN;
  const crumbs = folderCrumbs(folder, downloadsFolder);

  if (state === "completed") {
    const bytes = total ?? downloaded;
    return (
      <div className="glow-done flex h-screen flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none">
        {titleBar && <SmallTitleBar status="Done" onClose={closeNow} />}
        <div className="mt-4 flex flex-col items-center">
          <div className="relative h-[77px] w-[70px] self-center">
            <FileIcon name={iconName} className="h-[71px] w-[60px]" />
            <span className="absolute top-[51px] left-[44px] flex size-[26px] items-center justify-center rounded-full border-[3px] border-[#0b0e1a] bg-[#12b886] shadow-[0_0_14px_rgb(18_184_134/0.6)]">
              <Icon name="check" strokeWidth={3} className="size-3.5 text-white" />
            </span>
          </div>
          <h1 className="mt-2 text-lg font-semibold tracking-[-0.01em]">Download complete</h1>
          <p className="mt-1 max-w-full truncate text-[13px] text-[#9aa3b8]" title={name}>
            {name}
          </p>
          <div className="mt-3.5 flex gap-1.5">
            <Chip>{formatBytes(bytes)}</Chip>
            {took != null && took >= 1 && <Chip>{formatElapsed(took)}</Chip>}
            {took != null && took >= 1 && <Chip>avg {formatSpeed(bytes / took)}</Chip>}
          </div>
          <button
            type="button"
            title={folder}
            onClick={() => act(api.showInFolder(id))}
            className="mt-3.5 flex max-w-full items-center gap-1.5 rounded-lg border border-white/[0.08] bg-white/[0.03] px-3 py-1.5 hover:bg-white/[0.06]"
          >
            <Icon name="folder" className="size-3 text-[#8ea4ff]" />
            <Crumbs crumbs={crumbs} size="text-xs" />
          </button>
        </div>
        {error && (
          <p role="alert" className="mt-3 text-center text-xs text-[#ff9a9e]">
            {error}
          </p>
        )}
        <div className="mt-auto flex items-center gap-2">
          <button
            type="button"
            onClick={() => act(api.showInFolder(id))}
            className="mr-auto flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-[13px] font-semibold text-[#c9cfdd] hover:bg-white/[0.06]"
          >
            <Icon name="folder-open" />
            Show in folder
          </button>
          <button
            type="button"
            onClick={closeNow}
            className="h-9 rounded-[10px] border border-white/[0.12] bg-white/[0.04] px-3.5 text-[13px] font-semibold hover:bg-white/[0.08]"
          >
            Close
          </button>
          <button
            type="button"
            autoFocus
            onClick={() => act(api.openFile(id)).then(closeNow)}
            className="button-primary flex h-9 items-center gap-3 rounded-[10px] pr-2 pl-4 text-[13px] font-semibold"
          >
            Open file
            <EnterHint />
          </button>
        </div>
      </div>
    );
  }

  const status: Record<Exclude<State, "completed">, { label: string; dot: string; text: string }> = {
    starting: { label: "Starting…", dot: "bg-[#6b88ff]", text: "text-[#a9bbff]" },
    running: { label: "Downloading", dot: "bg-[#6b88ff]", text: "text-[#a9bbff]" },
    paused: { label: "Paused", dot: "bg-[#8b93a9]", text: "text-[#c9cfdd]" },
    failed: { label: "Failed", dot: "bg-[#f2555a]", text: "text-[#ff9a9e]" },
  };
  const shown = status[state];
  const stopped = state === "paused" || state === "failed";

  return (
    <div className="glow-window relative flex h-screen flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none">
      {titleBar && (
        <SmallTitleBar
          status={state === "failed" ? "Failed" : state === "paused" ? "Paused" : `${percent}% downloaded`}
          onClose={requestClose}
        />
      )}

      <div className="mt-5 flex flex-col gap-5 px-1">
        <div className="flex items-center gap-3.5">
          <FileIcon name={iconName} className="h-[52px] w-11" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <h1 className="truncate text-[15px] font-semibold" title={name}>
              {name}
            </h1>
            <p className="flex items-center gap-1.5 text-xs" aria-live="polite">
              <span className={`size-1.5 rounded-full ${shown.dot}`} />
              <span className={`font-medium ${shown.text}`}>{shown.label}</span>
            </p>
          </div>
          <p className="flex items-start gap-0.5 self-start tabular-nums" aria-label={`${percent} percent`}>
            <span className="text-[40px] leading-none font-light tracking-[-0.03em] text-white">{percent}</span>
            <span className="text-lg text-[#8b93a9]">%</span>
          </p>
        </div>

        <ProgressBar fraction={fraction} state={state} />

        <div className="overflow-hidden rounded-xl border border-white/[0.08] bg-white/[0.03]">
          <dl className="grid grid-cols-3 pt-3.5 tabular-nums">
            <Stat label="Downloaded">
              {formatBytes(downloaded)}
              {total != null && <span className="ml-1 text-xs font-normal text-[#8b93a9]">/ {formatBytes(total)}</span>}
            </Stat>
            <Stat label="Speed" divider>
              {state === "running" ? formatSpeed(speed) : "—"}
            </Stat>
            <Stat label="Time left" divider>
              {Number.isFinite(left) ? formatElapsed(left) : "—"}
            </Stat>
          </dl>
          <SpeedGraph samples={samples} className="mt-3.5 h-12 w-full" />
        </div>
      </div>

      {error && (
        <p role="alert" className="mt-3 truncate px-1 text-xs text-[#ff9a9e]" title={error}>
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center justify-end gap-2">
        {stopped ? (
          <button
            type="button"
            onClick={() => {
              setError(null);
              setState("starting");
              act(api.resume([id]));
            }}
            className="flex h-9 items-center gap-1.5 rounded-[10px] border border-white/[0.12] bg-white/[0.04] px-3 text-[13px] font-semibold hover:bg-white/[0.08]"
          >
            <Icon name={state === "failed" ? "retry" : "play"} className="text-[#c9cfdd]" />
            {state === "failed" ? "Retry" : "Resume"}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => act(api.pause([id]))}
            className="flex h-9 items-center gap-1.5 rounded-[10px] border border-white/[0.12] bg-white/[0.04] px-3 text-[13px] font-semibold hover:bg-white/[0.08]"
          >
            <Icon name="pause" className="text-[#c9cfdd]" />
            Pause
          </button>
        )}
        <button
          type="button"
          onClick={requestClose}
          className="h-9 rounded-[10px] border border-[#e5484d55] bg-[#e5484d1f] px-4 text-[13px] font-semibold text-[#ff9a9e] hover:bg-[#e5484d33]"
        >
          Cancel
        </button>
      </div>

      {confirmCancel && (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="cancel-title"
          aria-describedby="cancel-desc"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/65 px-6 backdrop-blur-sm"
        >
          <div className="w-full max-w-sm rounded-2xl border border-white/[0.12] bg-[#12172a] p-5 shadow-[0_24px_48px_rgb(0_0_0/0.65)]">
            <h2 id="cancel-title" className="text-[15px] font-semibold text-[#f4f6fb]">
              Cancel this download?
            </h2>
            <p id="cancel-desc" className="mt-1.5 text-xs leading-relaxed text-[#9aa3b8]">
              Closing this window will stop and remove <span className="font-medium text-[#d5dae6]">{name}</span>.
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                disabled={cancelling}
                onClick={() => setConfirmCancel(false)}
                className="h-8 rounded-lg border border-white/[0.12] bg-white/[0.05] px-3 text-xs font-semibold text-[#f4f6fb] hover:bg-white/[0.1] disabled:opacity-50"
              >
                Keep downloading
              </button>
              <button
                type="button"
                autoFocus
                disabled={cancelling}
                onClick={cancelAndClose}
                className="h-8 rounded-lg bg-[#e5484d] px-3.5 text-xs font-semibold text-white hover:bg-[#f2555a] disabled:opacity-50"
              >
                {cancelling ? "Cancelling…" : "Yes, cancel"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function Stat({ label, children, divider }: { label: string; children: ReactNode; divider?: boolean }) {
  return (
    <div className="flex items-center">
      {divider && <span className="h-9 w-px shrink-0 bg-white/[0.08]" />}
      <div className="flex min-w-0 flex-col gap-1 pl-4">
        <dt className="text-xs text-[#8b93a9]">{label}</dt>
        <dd className="flex items-baseline text-[17px] font-semibold whitespace-nowrap">{children}</dd>
      </div>
    </div>
  );
}

function ProgressBar({
  fraction,
  state,
}: {
  fraction: number;
  state: State;
}) {
  const fill =
    state === "paused" ? "bg-[#6b7390]" : state === "failed" ? "bg-[#e5484d]" : "bar-fill";
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.floor(fraction * 100)}
      aria-label="Progress"
      className="h-2 rounded-full bg-white/[0.08]"
    >
      <div
        className={`h-full rounded-full transition-[width] duration-300 ${fill}`}
        style={{ width: `${fraction * 100}%` }}
      />
    </div>
  );
}
