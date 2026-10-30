// SPDX-License-Identifier: GPL-3.0-or-later

// The window Windows' Apps list opens to remove OmniDM (OmniDm.exe --uninstall,
// app/src-tauri/src/uninstall.rs): it asks, shows the silent uninstaller's progress,
// then says it is done.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../components/Fields";
import { Icon } from "../components/Icon";
import { closeWindow } from "../components/WindowMotion";
import { errorText } from "../lib/api";
import { formatBytes } from "../lib/format";
import { Badge, CheckRow, Heading, Panel, PathText, ProgressBar, SetupFrame, type StepState, StepRow } from "./parts";

type UninstallInfo = { folder: string; size: number };

/** Sent while the uninstaller runs: `close` until OmniDM is closed, then `files`. */
type Progress = { phase: "close" | "files"; fraction: number };

type Screen = "ask" | "removing" | "failed" | "done";

export function UninstallWindow() {
  const [info, setInfo] = useState<UninstallInfo | null>(null);
  const [deleteData, setDeleteData] = useState(false);
  const [screen, setScreen] = useState<Screen>("ask");
  const [progress, setProgress] = useState<Progress>({ phase: "close", fraction: 0 });
  /** Every step ticked: the uninstaller has finished. */
  const [finished, setFinished] = useState(false);
  const [error, setError] = useState("");
  const startedAt = useRef(0);

  useEffect(() => {
    void invoke<UninstallInfo>("uninstall_info").then(setInfo);
    const stop = listen<Progress>("uninstall-progress", (event) => setProgress(event.payload));
    return () => void stop.then((f) => f());
  }, []);

  const remove = async () => {
    setError("");
    setFinished(false);
    setProgress({ phase: "close", fraction: 0 });
    setScreen("removing");
    startedAt.current = Date.now();
    try {
      await invoke("uninstall", { deleteData });
      // Long enough to see the steps tick, even when it took a second.
      const shown = Date.now() - startedAt.current;
      await new Promise((done) => setTimeout(done, Math.max(0, 1400 - shown)));
      setFinished(true);
      setProgress({ phase: "files", fraction: 1 });
      await new Promise((done) => setTimeout(done, 600));
      setScreen("done");
    } catch (e) {
      setError(errorText(e));
      setScreen("failed");
    }
  };

  if (screen === "done") {
    return (
      <SetupFrame
        status="Uninstall"
        glow="green"
        footer={
          <>
            <span className="flex-1" />
            <Button primary autoFocus onClick={() => void closeWindow()}>
              Close
            </Button>
          </>
        }
      >
        <Heading
          title="OmniDM is removed"
          lead={
            <>
              {deleteData ? "Your download list and settings were deleted too." : "Your download list and settings are kept,"}
              <br />
              {deleteData
                ? "Files you downloaded stay where they are."
                : "in case you install OmniDM again. Downloaded files stay too."}
            </>
          }
          badge={<Badge icon="check" tone="green" />}
        />
        <p className="flex items-start gap-1.5 text-[13px] text-[#8b93a9]">
          <Icon name="puzzle" className="mt-0.5 size-3.5 shrink-0" />
          The browser extension can't reach OmniDM any more. Remove it from your browser's extensions page.
        </p>
      </SetupFrame>
    );
  }

  if (screen === "removing" || screen === "failed") {
    const failed = screen === "failed";
    const steps: StepState[] = finished
      ? ["done", "done", "done"]
      : progress.phase === "close"
        ? ["now", "next", "next"]
        : ["done", "now", "next"];
    const phaseText = progress.phase === "close" ? "Closing OmniDM" : "Removing files";
    return (
      <SetupFrame
        status="Uninstall"
        glow="red"
        // The uninstaller can't be stopped halfway; closing waits for it.
        onClose={failed ? undefined : () => {}}
        footer={
          <>
            <span className="flex-1" />
            {failed ? (
              <>
                <Button onClick={() => void closeWindow()}>Close</Button>
                <Button danger autoFocus onClick={() => void remove()}>
                  <Icon name="retry" />
                  Try again
                </Button>
              </>
            ) : (
              <Button disabled title="Removing can't be stopped halfway">
                Cancel
              </Button>
            )}
          </>
        }
      >
        <div className="flex flex-col gap-1">
          <h1 className="text-xl font-semibold tracking-[-0.02em]">
            {failed ? "OmniDM wasn't removed" : "Removing OmniDM"}
          </h1>
          <p className="text-[13px] text-[#8b93a9]">
            {failed ? "Try again, or close this and remove it later." : "This takes a few seconds."}
          </p>
        </div>
        {failed ? (
          <p
            role="alert"
            className="flex items-start gap-2 rounded-[14px] border border-[#e5484d4d] bg-[#e5484d14] px-3.5 py-3 text-[13px] text-[#ffc9cb]"
          >
            <Icon name="info" className="mt-0.5 size-3.5 shrink-0 text-[#ff9a9e]" />
            <span className="select-text">{error}</span>
          </p>
        ) : (
          <>
            <div className="flex flex-col gap-2.5">
              <div className="flex items-center text-[13px] font-semibold">
                <span className="flex-1">{finished ? "Done" : phaseText}</span>
                <span className="font-mono">{Math.round(progress.fraction * 100)}%</span>
              </div>
              <ProgressBar fraction={progress.fraction} label="Uninstall progress" />
              {info && (
                <span className="flex min-w-0 items-center gap-2 font-mono text-xs text-[#8b93a9]">
                  <span className="shrink-0">Removing</span>
                  <PathText path={info.folder} className="text-xs" />
                </span>
              )}
            </div>
            <Panel>
              <StepRow label="Unlink the browser and close OmniDM" state={steps[0]} />
              <StepRow label="Remove app files and shortcuts" state={steps[1]} />
              <StepRow
                label={deleteData ? "Delete download list and settings" : "Remove it from Windows' Apps list"}
                state={steps[2]}
              />
            </Panel>
          </>
        )}
      </SetupFrame>
    );
  }

  return (
    <SetupFrame
      status="Uninstall"
      glow="red"
      footer={
        <>
          <span className="flex-1" />
          <Button autoFocus onClick={() => void closeWindow()}>
            Cancel
          </Button>
          <Button danger onClick={() => void remove()} disabled={!info}>
            <Icon name="trash" />
            Uninstall
          </Button>
        </>
      }
    >
      <Heading
        title="Remove OmniDM?"
        lead={
          <>
            The app, its shortcuts and the browser extension's link are removed.
            <br />
            Files you downloaded stay where they are.
          </>
        }
        badge={<Badge icon="trash" tone="red" />}
      />
      <Panel>
        <CheckRow
          icon="database"
          label="Also delete my download list and settings"
          hint="History, queues, categories and settings. Unfinished downloads can't be resumed."
          checked={deleteData}
          onChange={setDeleteData}
        />
      </Panel>
      <div className="flex flex-col gap-1.5 text-[13px] text-[#8b93a9]">
        <span className="flex items-center gap-1.5">
          <Icon name="info" className="size-3.5 shrink-0" />
          If OmniDM is running, it closes first.
        </span>
        {info && (
          <span className="flex min-w-0 items-center gap-1.5 pl-5">
            <span className="shrink-0">Frees {formatBytes(info.size)} in</span>
            <PathText path={info.folder} className="text-xs" />
          </span>
        )}
      </div>
    </SetupFrame>
  );
}
