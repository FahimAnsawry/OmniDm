// SPDX-License-Identifier: GPL-3.0-or-later

// Add download: a link (taken from the clipboard when one was copied), where to save it,
// and whether to start now or later in a queue.

import { useEffect, useRef, useState } from "react";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { Button, Field, FolderField, inputClass, selectClass } from "./Fields";

const CONNECTIONS = [1, 2, 4, 8, 16];

function isWebLink(text: string): boolean {
  try {
    const url = new URL(text.trim());
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

export function AddDialog({ queues, onClose }: { queues: QueueSummary[]; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  /** The user chose the folder; the link's category no longer changes it. */
  const [folderChosen, setFolderChosen] = useState(false);
  const [category, setCategory] = useState<string | undefined>();
  const [connections, setConnections] = useState("");
  const [queue, setQueue] = useState(queues[0]?.name ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    dialog.current?.showModal();
    readText()
      .then((text) => {
        if (isWebLink(text)) setUrl((current) => current || text.trim());
      })
      .catch(() => {
        // Nothing (or no text) on the clipboard.
      });
  }, []);

  // The link's category decides the folder, until the user picks one.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      api
        .suggestPlace(isWebLink(url) ? url : "", name.trim() || undefined)
        .then((place) => {
          setCategory(place.category);
          if (!folderChosen) setFolder(place.folder);
        })
        .catch(() => {});
    }, 250);
    return () => window.clearTimeout(timer);
  }, [url, name, folderChosen]);

  const add = async (inQueue?: string) => {
    if (!isWebLink(url)) {
      setError("Enter a link that starts with http:// or https://");
      return;
    }
    setBusy(true);
    try {
      await api.addDownload({
        url,
        fileName: name.trim() || undefined,
        folder,
        connections: connections ? Number(connections) : undefined,
        queue: inQueue,
      });
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      aria-labelledby="add-title"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className="m-auto w-[34rem] max-w-[calc(100vw-2rem)] rounded-xl border border-neutral-200 bg-white p-0 text-neutral-900 shadow-2xl backdrop:bg-black/30 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
        className="space-y-3.5 p-5"
      >
        <h2 id="add-title" className="text-[15px] font-semibold">
          Add download
        </h2>
        <Field label="Link" htmlFor="add-url">
          <input
            id="add-url"
            value={url}
            onChange={(e) => {
              setUrl(e.target.value);
              setError(null);
            }}
            placeholder="https://example.com/file.zip"
            spellCheck={false}
            autoFocus
            className={inputClass}
          />
        </Field>
        <div className="flex gap-3">
          <div className="min-w-0 flex-1">
            <Field label="File name" htmlFor="add-name">
              <input
                id="add-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="The server's name"
                spellCheck={false}
                className={inputClass}
              />
            </Field>
          </div>
          <Field label="Connections" htmlFor="add-connections">
            <select
              id="add-connections"
              value={connections}
              onChange={(e) => setConnections(e.target.value)}
              className={`${selectClass} w-24`}
            >
              <option value="">Auto</option>
              {CONNECTIONS.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label={category ? `Save in (${category})` : "Save in"} htmlFor="add-folder">
          <FolderField
            id="add-folder"
            value={folder}
            onChange={(f) => {
              setFolder(f);
              setFolderChosen(true);
            }}
          />
        </Field>
        {error && (
          <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
            {error}
          </p>
        )}
        <div className="flex items-center justify-end gap-2 pt-1">
          <Button onClick={onClose} className="mr-auto">
            Cancel
          </Button>
          {queues.length > 0 && (
            <div className="flex">
              <Button
                onClick={() => add(queue)}
                disabled={busy}
                className="rounded-r-none"
                title={`Add to ${queue} without starting`}
              >
                Later
              </Button>
              <select
                aria-label="Queue"
                value={queue}
                onChange={(e) => setQueue(e.target.value)}
                className={`${selectClass} -ml-px max-w-32 rounded-l-none`}
              >
                {queues.map((q) => (
                  <option key={q.queueId} value={q.name}>
                    {q.name}
                  </option>
                ))}
              </select>
            </div>
          )}
          <Button type="submit" primary disabled={busy}>
            Download now
          </Button>
        </div>
      </form>
    </dialog>
  );
}
