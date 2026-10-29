// SPDX-License-Identifier: GPL-3.0-or-later

// The capture prompt: its own small window, opened when a download is taken from the
// browser and "Ask before downloading" is on.

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AskAbout } from "../../../protocol/ts/AskAbout";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { formatBytes } from "../lib/format";
import { DownloadProgress } from "./DownloadProgress";
import { Button, Field, FolderField, inputClass, selectClass } from "./Fields";
import { Icon } from "./Icon";

const close = () => getCurrentWindow().close();

export function CapturePrompt({ id }: { id: number }) {
  const [about, setAbout] = useState<AskAbout | null>(null);
  const [categories, setCategories] = useState<CategorySummary[]>([]);
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [category, setCategory] = useState("");
  const [queue, setQueue] = useState("");
  const [dontAsk, setDontAsk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Started with "Download now": the window shows its progress from then on. */
  const [started, setStarted] = useState(false);

  useEffect(() => {
    Promise.all([api.capturePrompt(id), api.listCategories(), api.listQueues()])
      .then(([found, categoryList, queueList]) => {
        // Answered elsewhere, or deleted from the list.
        if (!found) return close();
        setAbout(found);
        setName(found.fileName);
        setFolder(found.folder);
        setCategory(found.category ?? "");
        setCategories(categoryList);
        setQueues(queueList);
        setQueue(queueList[0]?.name ?? "");
      })
      .catch((e) => setError(errorText(e)));
  }, [id]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (started) close();
        else cancel();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });

  const pickCategory = (name: string) => {
    setCategory(name);
    const chosen = categories.find((c) => c.name === name);
    if (about) setFolder(chosen?.folder ?? about.downloadsFolder);
  };

  const answer = async (inQueue?: string) => {
    setBusy(true);
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

  if (started) {
    return <DownloadProgress id={id} name={name.trim()} size={about?.size} />;
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        answer();
      }}
      className="flex h-screen flex-col gap-3.5 bg-white p-5 font-sans text-neutral-900 select-none dark:bg-neutral-900 dark:text-neutral-100"
    >
      <div className="flex items-start gap-3">
        <span className="rounded-lg bg-sky-100 p-2 text-sky-700 dark:bg-sky-900/50 dark:text-sky-300">
          <Icon name="downloading" className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="text-[15px] font-semibold">Download this file?</h1>
          <p className="truncate text-xs text-neutral-500 select-text" title={about?.url}>
            {about?.url ?? "Loading…"}
          </p>
        </div>
        {about && (
          <div className="text-right text-xs text-neutral-500">
            <p className="text-[13px] font-medium text-neutral-800 tabular-nums dark:text-neutral-200">
              {about.size != null ? formatBytes(about.size) : "Size unknown"}
            </p>
            <p>{about.resumable ? "Can pause and resume" : "Can't resume if stopped"}</p>
          </div>
        )}
      </div>

      <Field label="File name" htmlFor="name">
        <input
          id="name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          spellCheck={false}
          className={inputClass}
        />
      </Field>
      <div className="flex gap-3">
        <Field label="Category" htmlFor="category">
          <select
            id="category"
            value={category}
            onChange={(e) => pickCategory(e.target.value)}
            className={`${selectClass} w-36`}
          >
            <option value="">None</option>
            {categories.map((c) => (
              <option key={c.categoryId} value={c.name}>
                {c.name}
              </option>
            ))}
          </select>
        </Field>
        <div className="min-w-0 flex-1">
          <Field label="Save in" htmlFor="folder">
            <FolderField id="folder" value={folder} onChange={setFolder} />
          </Field>
        </div>
      </div>

      {error && (
        <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center gap-2">
        <label className="mr-auto flex items-center gap-2 text-xs text-neutral-600 dark:text-neutral-400">
          <input
            type="checkbox"
            checked={dontAsk}
            onChange={(e) => setDontAsk(e.target.checked)}
            className="size-3.5 accent-sky-600"
          />
          Don't ask again
        </label>
        <Button onClick={cancel} disabled={busy}>
          Cancel
        </Button>
        {queues.length > 0 && (
          <div className="flex">
            <Button
              onClick={() => answer(queue)}
              disabled={busy || !about}
              className="rounded-r-none"
              title={`Add to ${queue} without starting`}
            >
              Later
            </Button>
            <select
              aria-label="Queue"
              value={queue}
              onChange={(e) => setQueue(e.target.value)}
              className={`${selectClass} -ml-px max-w-28 rounded-l-none`}
            >
              {queues.map((q) => (
                <option key={q.queueId} value={q.name}>
                  {q.name}
                </option>
              ))}
            </select>
          </div>
        )}
        <Button type="submit" primary autoFocus disabled={busy || !about}>
          Download now
        </Button>
      </div>
    </form>
  );
}
