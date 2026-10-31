// SPDX-License-Identifier: GPL-3.0-or-later

// The update window: opens when a newer OmniDM is out. "Update now" turns it into the
// updating card (a bar shows how far), which pauses the downloads and restarts into the new
// version; "Later" asks again at the next look. After the restart the same window opens as
// the finished card, saying what changed.

import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorText, type UpdateInfo } from "../lib/api";
import { formatBytes } from "../lib/format";
import { Chip, EnterHint, Logo, SmallTitleBar } from "./Chrome";
import { Icon } from "./Icon";
import { closeWindow } from "./WindowMotion";

type Stage = "offer" | "busy";

export function UpdateWindow() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [stage, setStage] = useState<Stage>("offer");
  const [done, setDone] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      api
        .updateInfo()
        .then(setInfo)
        .catch((e) => setError(errorText(e)));
    load();
    // A newer version found while the window is open.
    const found = listen("update-found", load);
    const downloaded = listen<{ done: number; total: number | null }>("update-downloaded", (event) => {
      setDone(event.payload.done);
      setTotal(event.payload.total);
    });
    return () => {
      found.then((stop) => stop());
      downloaded.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && stage === "offer") void closeWindow();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [stage]);

  const install = () => {
    setError(null);
    setStage("busy");
    setDone(0);
    setTotal(null);
    api.installUpdate().catch((e) => {
      setError(errorText(e));
      setStage("offer");
    });
  };

  const finished = info?.installed === true;
  const busy = !finished && stage !== "offer";
  const fraction = total ? Math.min(done / total, 1) : 0;
  // Downloaded: the downloads pause, then OmniDM closes for the installer.
  const restarting = total != null && done >= total;

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (finished) void closeWindow();
        else if (!busy && info) install();
      }}
      className="glow-window flex h-full flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none"
    >
      <SmallTitleBar
        status={finished ? "Updated" : busy ? "Updating" : "Update"}
        onClose={busy ? () => {} : undefined}
      />

      <div className="mt-5 flex min-h-0 flex-1 flex-col gap-[18px] px-1">
        <div className="flex items-center gap-4">
          <span className="relative flex size-14 shrink-0 items-center justify-center rounded-[14px] border border-white/[0.1] bg-white/[0.05]">
            <Logo className="size-8" />
            {finished && (
              <span className="absolute -right-1.5 -bottom-1.5 flex size-6 items-center justify-center rounded-full border-2 border-[#0c0f17] bg-[#2fd3a0] text-[#0c0f17]">
                <Icon name="check" className="size-3.5" />
              </span>
            )}
          </span>
          <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
            <h1 className="text-base font-semibold tracking-[-0.01em]">
              {!info
                ? "Looking for the update…"
                : finished
                  ? `OmniDM ${info.version} is installed`
                  : busy
                    ? `Updating to OmniDM ${info.version}`
                    : `OmniDM ${info.version} is ready`}
            </h1>
            {info && (
              <div className="flex gap-1.5">
                {finished ? (
                  <>
                    <Chip tone="green">Up to date</Chip>
                    {info.previous && <Chip>Was {info.previous}</Chip>}
                  </>
                ) : (
                  <>
                    <Chip tone="green">{busy ? "Updating" : "New"}</Chip>
                    <Chip>{busy ? `${info.current} → ${info.version}` : `You have ${info.current}`}</Chip>
                    {info.date && <Chip>{releaseDay(info.date)}</Chip>}
                  </>
                )}
              </div>
            )}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-white/[0.08] bg-white/[0.03] px-3.5 py-3">
          <p className="mb-1.5 text-xs font-semibold tracking-[0.06em] text-[#8b93a9] uppercase">
            {info ? `What's new in ${info.version}` : "What's new"}
          </p>
          <p className="text-[13px] leading-relaxed whitespace-pre-line text-[#c9cfdd] select-text">
            {info?.notes?.trim() || "Fixes and improvements."}
          </p>
        </div>

        {busy ? (
          <div className="flex flex-col gap-2">
            <div
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.floor(fraction * 100)}
              aria-label="Update download"
              className="relative h-2 overflow-hidden rounded-full bg-white/[0.08]"
            >
              {total ? (
                <div
                  className="bar-fill h-full rounded-full transition-[width] duration-300"
                  style={{ width: `${fraction * 100}%` }}
                />
              ) : (
                <div className="bar-searching" />
              )}
            </div>
            <p className="flex items-center gap-1.5 text-xs text-[#8b93a9] tabular-nums">
              <Icon name="loader" className="size-3 animate-spin" />
              {restarting
                ? "Pausing your downloads and restarting OmniDM…"
                : `Downloading the update · ${formatBytes(done)}${total ? ` of ${formatBytes(total)}` : ""}`}
            </p>
          </div>
        ) : finished ? (
          <p className="flex items-center gap-1.5 text-xs text-[#8b93a9]">
            <Icon name="check" className="size-3 text-[#2fd3a0]" />
            The update is installed. Downloads that were running have carried on.
          </p>
        ) : (
          <p className="flex items-center gap-1.5 text-xs text-[#8b93a9]">
            <Icon name="info" className="size-3 text-[#6b7390]" />
            OmniDM restarts to update. Running downloads pause and then continue by themselves.
          </p>
        )}
      </div>

      {error && (
        <p role="alert" className="mt-3 truncate px-1 text-xs text-[#ff9a9e]" title={error}>
          {error}
        </p>
      )}

      <div className="mt-4 flex items-center justify-end gap-2">
        {finished ? (
          <button
            type="submit"
            autoFocus
            className="button-primary flex h-9 items-center gap-3 rounded-[10px] pr-2 pl-4 text-[13px] font-semibold"
          >
            Done
            <EnterHint />
          </button>
        ) : (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => void closeWindow()}
              className="button-glass flex h-9 items-center gap-1.5 rounded-[10px] px-3.5 text-[13px] font-semibold disabled:opacity-40"
            >
              <Icon name="clock" className="text-[#c9cfdd]" />
              Later
            </button>
            <button
              type="submit"
              autoFocus
              disabled={busy || !info}
              className="button-primary flex h-9 items-center gap-3 rounded-[10px] pr-2 pl-4 text-[13px] font-semibold disabled:opacity-50"
            >
              Update now
              <EnterHint />
            </button>
          </>
        )}
      </div>
    </form>
  );
}

/** "30 Oct 2026" from seconds since 1970. */
function releaseDay(seconds: number): string {
  return new Date(seconds * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}
