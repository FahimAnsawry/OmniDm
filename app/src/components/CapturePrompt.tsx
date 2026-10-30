// SPDX-License-Identifier: GPL-3.0-or-later

// The capture prompt: its own small window, opened when a download is taken from the
// browser and "Ask before downloading" is on.

import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import type { AskAbout } from "../../../protocol/ts/AskAbout";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { fileKind } from "../lib/fileKind";
import { formatBytes } from "../lib/format";
import { Chip, Crumbs, EnterHint, FileIcon, SmallTitleBar, folderCrumbs } from "./Chrome";
import { DownloadProgress } from "./DownloadProgress";
import { Icon } from "./Icon";
import { closeWindow } from "./WindowMotion";

const close = () => closeWindow();

/** "releases.ubuntu.com/24.04.1/": where the file comes from, without its name. */
function source(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.host + parsed.pathname.replace(/[^/]*$/, "");
  } catch {
    return url;
  }
}

export function CapturePrompt({ id }: { id: number }) {
  const [about, setAbout] = useState<AskAbout | null>(null);
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [folderChosen, setFolderChosen] = useState(false);
  const [category, setCategory] = useState("");
  const [dontAsk, setDontAsk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [laterMenu, setLaterMenu] = useState(false);
  const nameInput = useRef<HTMLInputElement>(null);
  /** Started with "Download now": the window shows its progress from then on. */
  const [started, setStarted] = useState(false);

  useEffect(() => {
    Promise.all([api.capturePrompt(id), api.listQueues()])
      .then(([found, queueList]) => {
        // Answered elsewhere, or deleted from the list.
        if (!found) return close();
        setAbout(found);
        setName(found.fileName);
        setFolder(found.folder);
        setCategory(found.category ?? "");
        setQueues(queueList);
      })
      .catch((e) => setError(errorText(e)));
  }, [id]);

  // Auto-select category and folder based on the file extension when the name changes.
  useEffect(() => {
    if (!about || !name.trim() || name === about.fileName) return;
    api
      .suggestPlace(about.url, name.trim())
      .then((place) => {
        setCategory(place.category ?? "");
        if (!folderChosen) setFolder(place.folder);
      })
      .catch(() => {});
  }, [about, name, folderChosen]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (laterMenu) setLaterMenu(false);
        else if (started) close();
        else cancel();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: folder || undefined, title: "Save in" });
    if (typeof picked === "string") {
      setFolder(picked);
      setFolderChosen(true);
    }
  };

  const answer = async (inQueue?: string) => {
    setBusy(true);
    setLaterMenu(false);
    try {
      await api.answerCapture({
        downloadId: id,
        fileName: name,
        folder,
        category: category || undefined,
        queue: inQueue,
      });
      if (dontAsk) await api.setAskBeforeDownload(false);
      if (inQueue) {
        await close();
      } else {
        // Now an ordinary window that can go behind others.
        await getCurrentWindow().setAlwaysOnTop(false).catch(() => {});
        setStarted(true);
      }
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const cancel = async () => {
    try {
      await api.cancelCapture(id);
    } finally {
      await close();
    }
  };

  if (started && about) {
    return (
      <DownloadProgress
        id={id}
        name={name.trim()}
        size={about.size}
        folder={folder}
        downloadsFolder={about.downloadsFolder}
      />
    );
  }

  const kind = fileKind(name);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        answer();
      }}
      className="glow-window flex h-full flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none"
    >
      <SmallTitleBar />

      <div className="mt-5 flex flex-col gap-[18px] px-1">
        <div className="flex items-center gap-4">
          <FileIcon name={name} className="size-14" />
          <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
            <div className="flex max-w-full items-center gap-2">
              <input
                ref={nameInput}
                aria-label="File name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                spellCheck={false}
                className="-mx-1.5 max-w-full min-w-10 truncate rounded-md border border-transparent bg-transparent px-1.5 text-base font-semibold tracking-[-0.01em] [field-sizing:content] outline-none hover:border-white/10 focus:border-[#6b88ff80] focus:bg-white/[0.04]"
              />
              <button
                type="button"
                aria-label="Rename"
                title="Rename"
                onClick={() => {
                  nameInput.current?.focus();
                  nameInput.current?.select();
                }}
                className="shrink-0 rounded p-0.5 text-[#6b7390] hover:text-[#c9cfdd]"
              >
                <Icon name="pencil" className="size-3" />
              </button>
            </div>
            {about && (
              <div className="flex gap-1.5">
                <Chip>{about.size != null ? formatBytes(about.size) : "Size unknown"}</Chip>
                <Chip>{kind.label}</Chip>
                {about.resumable ? <Chip tone="green">Resumable</Chip> : <Chip>Can't resume</Chip>}
              </div>
            )}
            <p className="flex max-w-full items-center gap-1.5 text-xs text-[#8b93a9]" title={about?.url}>
              <Icon name="link-2" className="size-3 text-[#6b7390]" />
              <span className="truncate select-text">{about ? source(about.url) : "Loading…"}</span>
            </p>
          </div>
        </div>

        <div className="rounded-xl border border-white/[0.08] bg-white/[0.03]">
          <div className="flex h-[46px] items-center gap-3 px-3">
            <Icon name="folder" className="text-[#8ea4ff]" />
            <span className="w-16 shrink-0 text-[13px] text-[#8b93a9]">Save to</span>
            <span className="min-w-0 flex-1" title={folder}>
              <Crumbs crumbs={folderCrumbs(folder, about?.downloadsFolder)} />
            </span>
            <button
              type="button"
              onClick={browse}
              className="button-glass rounded-lg px-2.5 py-1.5 text-xs font-medium"
            >
              Change
            </button>
          </div>
        </div>
      </div>

      {error && (
        <p role="alert" className="mt-3 px-1 text-xs text-[#ff9a9e]">
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center gap-2">
        <label className="mr-auto flex cursor-pointer items-center gap-2 text-xs text-[#8b93a9]">
          <span className="relative flex size-4 items-center justify-center">
            <input
              type="checkbox"
              checked={dontAsk}
              onChange={(e) => setDontAsk(e.target.checked)}
              className="peer size-4 cursor-pointer appearance-none rounded border border-white/[0.22] bg-white/[0.03] checked:border-[#4a6cff] checked:bg-[#4a6cff] focus-visible:outline-2 focus-visible:outline-[#6b88ff]"
            />
            <Icon
              name="check"
              strokeWidth={3}
              className="pointer-events-none absolute size-3 text-white opacity-0 peer-checked:opacity-100"
            />
          </span>
          Don't ask again
        </label>
        <button
          type="button"
          disabled={busy}
          onClick={cancel}
          className="flex h-9 items-center rounded-[10px] border border-transparent px-3.5 text-[13px] font-semibold text-[#ff8a8f] hover:border-[#e5484d]/35 hover:bg-[#e5484d]/[0.14] hover:text-[#ffc9cb] disabled:opacity-40"
        >
          Cancel
        </button>
        {queues.length > 0 && (
          <div className="relative">
            <button
              type="button"
              aria-haspopup="menu"
              aria-expanded={laterMenu}
              disabled={busy || !about}
              onClick={() => setLaterMenu((shown) => !shown)}
              className="button-glass flex h-9 items-center gap-1.5 rounded-[10px] pr-2.5 pl-3 text-[13px] font-semibold disabled:opacity-40"
            >
              <Icon name="clock" className="text-[#c9cfdd]" />
              Later
              <Icon name="down" className="size-3 text-[#8b93a9]" />
            </button>
            {laterMenu && (
              <>
                <div className="fixed inset-0" onClick={() => setLaterMenu(false)} />
                <div
                  role="menu"
                  className="absolute right-0 bottom-full mb-2 min-w-52 rounded-xl border border-white/[0.12] bg-[#141a2e] p-1 shadow-[0_16px_40px_-8px_rgb(0_0_0/0.6)]"
                >
                  {queues.map((q) => (
                    <button
                      key={q.queueId}
                      type="button"
                      role="menuitem"
                      onClick={() => answer(q.name)}
                      className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] hover:bg-white/[0.07]"
                    >
                      <Icon name={q.schedule ? "moon" : "queue"} className="text-[#8b93a9]" />
                      <span className="flex-1">Add to {q.name}</span>
                      {q.nextStart && (
                        <span className="font-mono text-xs text-[#8b93a9] tabular-nums">{q.nextStart.slice(-5)}</span>
                      )}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        )}
        <button
          type="submit"
          autoFocus
          disabled={busy || !about}
          className="button-primary flex h-9 items-center gap-3 rounded-[10px] pr-2 pl-4 text-[13px] font-semibold disabled:opacity-50"
        >
          Download now
          <EnterHint />
        </button>
      </div>
    </form>
  );
}
