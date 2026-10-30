// SPDX-License-Identifier: GPL-3.0-or-later

// OmniDM-Setup.exe's window (installer/): welcome, options, installing and finished.
// The install itself is Tauri's NSIS installer, run silently by the setup program
// (installer/src/lib.rs); this page shows its progress.

import { type FormEvent, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { EnterHint, Logo } from "../components/Chrome";
import { Button } from "../components/Fields";
import { Icon } from "../components/Icon";
import { closeWindow } from "../components/WindowMotion";
import { errorText } from "../lib/api";
import { formatBytes } from "../lib/format";
import {
  Badge,
  CheckBox,
  CheckRow,
  Heading,
  Panel,
  PathText,
  ProgressBar,
  SetupFrame,
  type StepState,
  StepRow,
} from "./parts";

type SetupInfo = {
  version: string;
  /** What the installed app takes on disk, in bytes. */
  size: number;
  defaultFolder: string;
  /** An OmniDM already installed for this user. */
  installed: { folder: string; version: string } | null;
};

type Choices = { folder: string; desktop: boolean; startMenu: boolean; autostart: boolean };

/** Sent by the setup program while the installer runs. */
type Progress = { phase: "copy" | "setup"; fraction: number };

type Screen = "welcome" | "options" | "installing" | "failed" | "done";

/** A folder picked with Change gets its own OmniDm folder inside, like the default one. */
function intoAppFolder(folder: string): string {
  const trimmed = folder.replace(/[\\/]+$/, "");
  return /[\\/]omnidm$/i.test(trimmed) ? trimmed : `${trimmed}\\OmniDm`;
}

export function SetupWindow() {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [screen, setScreen] = useState<Screen>("welcome");
  const [choices, setChoices] = useState<Choices>({ folder: "", desktop: true, startMenu: true, autostart: false });
  const [free, setFree] = useState<number | null>(null);
  const [progress, setProgress] = useState<Progress>({ phase: "copy", fraction: 0 });
  /** The last step lights up a moment after the second: the installer does both at once. */
  const [lastStep, setLastStep] = useState(false);
  const [error, setError] = useState("");
  const [launch, setLaunch] = useState(true);
  const [whatsNew, setWhatsNew] = useState(false);
  /** The terms must be accepted before Next. */
  const [accepted, setAccepted] = useState(false);
  const startedAt = useRef(0);

  useEffect(() => {
    void invoke<SetupInfo>("setup_info").then((found) => {
      setInfo(found);
      setChoices((c) => ({ ...c, folder: found.installed?.folder ?? found.defaultFolder }));
    });
    const stop = listen<Progress>("install-progress", (event) => setProgress(event.payload));
    return () => void stop.then((f) => f());
  }, []);

  useEffect(() => {
    if (!choices.folder) return;
    void invoke<number | null>("free_space", { folder: choices.folder }).then(setFree);
  }, [choices.folder]);

  useEffect(() => {
    if (progress.phase !== "setup") return;
    const timer = setTimeout(() => setLastStep(true), 600);
    return () => clearTimeout(timer);
  }, [progress.phase]);

  const updating = info?.installed != null;
  const sameVersion = updating && info?.installed?.version === info?.version;
  const tooSmall = info != null && free != null && free < info.size;

  const changeFolder = async () => {
    const picked = await open({ directory: true, defaultPath: choices.folder, title: "Install OmniDM in" });
    if (typeof picked === "string") setChoices((c) => ({ ...c, folder: intoAppFolder(picked) }));
  };

  const install = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!info || tooSmall) return;
    setError("");
    setProgress({ phase: "copy", fraction: 0 });
    setLastStep(false);
    setScreen("installing");
    startedAt.current = Date.now();
    try {
      await invoke("install", { choices });
      // Long enough to see the steps tick, even when the install took a second.
      const shown = Date.now() - startedAt.current;
      await new Promise((done) => setTimeout(done, Math.max(0, 1400 - shown)));
      setLastStep(true);
      setProgress({ phase: "setup", fraction: 1 });
      await new Promise((done) => setTimeout(done, 500));
      setScreen("done");
    } catch (e) {
      setError(errorText(e));
      setScreen("failed");
    }
  };

  const cancel = async () => {
    await invoke("cancel_install", { folder: choices.folder }).catch(() => {});
    await closeWindow();
  };

  const finish = async (event?: FormEvent) => {
    event?.preventDefault();
    await invoke("finish", { folder: choices.folder, launch, whatsNew }).catch(() => {});
    await closeWindow();
  };

  if (!info) return <SetupFrame status="Setup" footer={null}>{null}</SetupFrame>;

  if (screen === "welcome") {
    return (
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (accepted && !tooSmall) setScreen("options");
        }}
        className="h-full"
      >
        <SetupFrame
          status="Setup"
          footer={
            <>
              <span className="font-mono text-xs text-[#8b93a9]">Version {info.version}</span>
              <span className="flex-1" />
              <Button onClick={() => void closeWindow()}>Cancel</Button>
              <Button primary type="submit" disabled={!accepted || tooSmall} className="gap-3 pr-2">
                Next
                <EnterHint />
              </Button>
            </>
          }
        >
          <Heading
            title={sameVersion ? "Reinstall OmniDM" : updating ? "Update OmniDM" : "Install OmniDM"}
            lead={
              updating ? (
                <>
                  Version {info.installed!.version} is installed. Your downloads and settings
                  <br />
                  stay as they are.
                </>
              ) : (
                <>
                  Downloads split into up to 32 parts at once, and pick up
                  <br />
                  where they stopped after a crash or a lost connection.
                </>
              )
            }
            badge={
              <span className="flex size-[68px] shrink-0 items-center justify-center rounded-[18px] border border-white/[0.1] bg-white/[0.04] shadow-[0_12px_32px_-8px_rgb(238_63_106/0.35)]">
                <Logo className="size-[42px]" />
              </span>
            }
          />
          <Panel>
            <div className="flex items-center gap-3 px-3.5 py-3">
              <Icon name="folder" className="text-[#8ea4ff]" />
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-xs font-medium text-[#8b93a9]">Install to</span>
                <PathText path={choices.folder} className="font-medium" />
              </span>
              {!updating && (
                <Button onClick={() => void changeFolder()} className="h-[30px] px-3 text-xs">
                  Change
                </Button>
              )}
            </div>
            <div className="flex items-center gap-3 px-3.5 py-3">
              <Icon name="hard-drive" className="text-[#8ea4ff]" />
              <span className="flex-1 text-[13px] font-medium text-[#8b93a9]">Space needed</span>
              <span className="font-mono text-[13px] font-semibold">{formatBytes(info.size)}</span>
              {free != null && (
                <span className={`text-[13px] ${tooSmall ? "text-[#ff9a9e]" : "text-[#8b93a9]"}`}>
                  · {formatBytes(free)} free on {choices.folder.slice(0, 2)}
                </span>
              )}
            </div>
          </Panel>
          {tooSmall ? (
            <p role="alert" className="flex items-center gap-1.5 text-[13px] text-[#ff9a9e]">
              <Icon name="info" className="size-3.5" />
              Not enough space on {choices.folder.slice(0, 2)}. Free some space, or choose another drive with Change.
            </p>
          ) : (
            <div className="flex items-center gap-2.5 text-[13px]">
              <CheckBox id="accept-terms" checked={accepted} onChange={setAccepted} />
              <label htmlFor="accept-terms" className="cursor-pointer text-[#c9cfdd]">
                I accept the terms and conditions
              </label>
              <span className="text-[#6b7390]">·</span>
              <button
                type="button"
                onClick={() => void invoke("open_page", { page: "licence" })}
                className="font-medium text-[#8ea4ff] hover:text-[#b9c8ff] hover:underline"
              >
                Read the GPL-3.0 licence
              </button>
            </div>
          )}
        </SetupFrame>
      </form>
    );
  }

  if (screen === "options") {
    const set = (part: Partial<Choices>) => setChoices((c) => ({ ...c, ...part }));
    return (
      <form onSubmit={install} className="h-full">
        <SetupFrame
          status="Setup"
          footer={
            <>
              <span className="font-mono text-xs text-[#8b93a9]">{formatBytes(info.size)} needed</span>
              <span className="flex-1" />
              <Button onClick={() => setScreen("welcome")}>
                <Icon name="back" className="text-[#c9cfdd]" />
                Back
              </Button>
              <Button primary type="submit" autoFocus disabled={tooSmall} className="gap-3 pr-2">
                {sameVersion ? "Reinstall" : updating ? "Update" : "Install"}
                <EnterHint />
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-1">
            <h1 className="text-xl font-semibold tracking-[-0.02em]">Install options</h1>
            <p className="text-[13px] text-[#8b93a9]">You can change these later in Windows or in Settings.</p>
          </div>
          <Panel>
            <CheckRow icon="monitor" label="Desktop shortcut" checked={choices.desktop} onChange={(desktop) => set({ desktop })} />
            <CheckRow
              icon="layout-grid"
              label="Start menu shortcut"
              checked={choices.startMenu}
              onChange={(startMenu) => set({ startMenu })}
            />
            <CheckRow
              icon="power"
              label="Start OmniDM when Windows starts"
              hint="Waits quietly in the tray, so downloads from the browser start at once"
              checked={choices.autostart}
              onChange={(autostart) => set({ autostart })}
            />
          </Panel>
        </SetupFrame>
      </form>
    );
  }

  if (screen === "done") {
    return (
      <form onSubmit={finish} className="h-full">
        <SetupFrame
          status="Setup"
          glow="green"
          footer={
            <>
              <span className="flex-1" />
              <Button primary type="submit" autoFocus className="gap-3 pr-2">
                Finish
                <EnterHint />
              </Button>
            </>
          }
        >
          <Heading
            title={updating ? "OmniDM is updated" : "OmniDM is installed"}
            lead={
              <>
                Add the extension to your browser. It connects to OmniDM
                <br />
                by itself whenever the app is running.
              </>
            }
            badge={<Badge icon="check" tone="green" />}
          />
          <Panel>
            <div className="flex items-center gap-3 px-3.5 py-3.5">
              <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-white/[0.1] bg-white/[0.05] text-[#8ea4ff]">
                <Icon name="puzzle" className="size-[18px]" />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-[13px] font-semibold">Browser extension</span>
                <span className="text-xs text-[#8b93a9]">Chrome, Edge, Brave and Firefox</span>
              </span>
              <Button onClick={() => void invoke("open_page", { page: "extension" })}>
                <Icon name="external-link" className="text-[#c9cfdd]" />
                Get extension
              </Button>
            </div>
          </Panel>
          <Panel>
            <CheckRow label="Launch OmniDM now" checked={launch} onChange={setLaunch} />
            <CheckRow label={`Show what's new in ${info.version}`} checked={whatsNew} onChange={setWhatsNew} />
          </Panel>
        </SetupFrame>
      </form>
    );
  }

  // Installing, or failed.
  const failed = screen === "failed";
  const steps: StepState[] =
    progress.phase === "copy" ? ["now", "next", "next"] : lastStep ? ["done", "done", "now"] : ["done", "now", "next"];
  const phaseText = progress.phase === "copy" ? "Copying files" : lastStep ? "Registering with Windows" : "Creating shortcuts";
  return (
    <SetupFrame
      status="Setup"
      glow={failed ? "red" : "blue"}
      // Closing mid-install stops it, like Cancel.
      onClose={failed ? undefined : () => void cancel()}
      footer={
        <>
          <span className="flex-1" />
          {failed ? (
            <>
              <Button onClick={() => void closeWindow()}>Close</Button>
              <Button primary autoFocus onClick={() => void install()}>
                <Icon name="retry" />
                Try again
              </Button>
            </>
          ) : (
            <Button
              onClick={() => void cancel()}
              disabled={updating}
              title={updating ? "An update can't be stopped halfway" : undefined}
            >
              Cancel
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-1">
        <h1 className="text-xl font-semibold tracking-[-0.02em]">
          {failed ? "OmniDM wasn't installed" : updating ? "Updating OmniDM" : "Installing OmniDM"}
        </h1>
        <p className="text-[13px] text-[#8b93a9]">
          {failed ? "Try again, or close this and run the setup later." : "This takes a few seconds. You can keep working."}
        </p>
      </div>
      {failed ? (
        <p role="alert" className="flex items-start gap-2 rounded-[14px] border border-[#e5484d4d] bg-[#e5484d14] px-3.5 py-3 text-[13px] text-[#ffc9cb]">
          <Icon name="info" className="mt-0.5 size-3.5 shrink-0 text-[#ff9a9e]" />
          <span className="select-text">{error}</span>
        </p>
      ) : (
        <div className="flex flex-col gap-2.5">
          <div className="flex items-center text-[13px] font-semibold">
            <span className="flex-1">{phaseText}</span>
            <span className="font-mono">{Math.round(progress.fraction * 100)}%</span>
          </div>
          <ProgressBar fraction={progress.fraction} label="Install progress" />
          <span className="flex min-w-0 items-center gap-2 font-mono text-xs text-[#8b93a9]">
            <span className="shrink-0">OmniDm.exe →</span>
            <PathText path={choices.folder} className="text-xs" />
          </span>
        </div>
      )}
      {!failed && (
        <Panel>
          <StepRow label="Copy app files" state={steps[0]} />
          <StepRow label="Create shortcuts" state={steps[1]} />
          <StepRow label="Register with Windows" state={steps[2]} />
        </Panel>
      )}
    </SetupFrame>
  );
}
