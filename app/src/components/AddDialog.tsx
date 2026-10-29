// SPDX-License-Identifier: GPL-3.0-or-later

// Add download: a link (taken from the clipboard when one was copied), where to save it,
// and whether to start now or later in a queue. A video's playlist link (HLS or DASH) is
// read first, to choose its quality.

import { useEffect, useRef, useState } from "react";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { looksLikeStream } from "../lib/media";
import { Button, Field, FolderField, inputClass, selectClass } from "./Fields";
import { MediaPicker } from "./MediaPicker";

const CONNECTIONS = [1, 2, 4, 8, 16];

/** Reading a video link's playlist. */
type MediaState = { status: "reading" } | { status: "ready"; info: MediaInfo } | { status: "failed"; error: string };

function isWebLink(text: string): boolean {
  try {
    const url = new URL(text.trim());
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

export function AddDialog({
  queues,
  initialUrl,
  onClose,
}: {
  queues: QueueSummary[];
  /** A link copied while the clipboard monitor was on. */
  initialUrl?: string;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [url, setUrl] = useState(initialUrl ?? "");
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  /** The user chose the folder; the link's category no longer changes it. */
  const [folderChosen, setFolderChosen] = useState(false);
  const [category, setCategory] = useState<string | undefined>();
  const [connections, setConnections] = useState("");
  const [queue, setQueue] = useState(queues[0]?.name ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const stream = looksLikeStream(url);
  const [media, setMedia] = useState<MediaState | null>(null);
  const [quality, setQuality] = useState(0);
  const [audio, setAudio] = useState<number | undefined>();
  const info = media?.status === "ready" ? media.info : undefined;

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

  // A video link is read once typing stops: what it offers, or why it can't be downloaded.
  useEffect(() => {
    if (!stream) {
      setMedia(null);
      return;
    }
    setMedia({ status: "reading" });
    let current = true;
    const timer = window.setTimeout(() => {
      api
        .lookAtMedia(url.trim())
        .then((found) => {
          if (!current) return;
          setMedia({ status: "ready", info: found });
          setQuality(0);
          setAudio(found.qualities[0]?.defaultAudio);
        })
        .catch((e) => current && setMedia({ status: "failed", error: errorText(e) }));
    }, 400);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [url, stream]);

  const chooseQuality = (index: number) => {
    setQuality(index);
    const sounds = info?.qualities[index]?.audio ?? [];
    setAudio((current) =>
      current != null && sounds.includes(current) ? current : info?.qualities[index]?.defaultAudio,
    );
  };

  // The link's category decides the folder, until the user picks one. A video is named
  // as an .mp4, so it lands with the videos.
  const videoName = stream ? `${name.trim() || info?.title || "video"}.mp4` : undefined;
  useEffect(() => {
    const timer = window.setTimeout(() => {
      api
        .suggestPlace(isWebLink(url) ? url : "", videoName ?? (name.trim() || undefined))
        .then((place) => {
          setCategory(place.category);
          if (!folderChosen) setFolder(place.folder);
        })
        .catch(() => {});
    }, 250);
    return () => window.clearTimeout(timer);
  }, [url, name, folderChosen, videoName]);

  const add = async (inQueue?: string) => {
    if (!isWebLink(url)) {
      setError("Enter a link that starts with http:// or https://");
      return;
    }
    if (stream && !info) {
      setError(media?.status === "failed" ? media.error : "Wait until the video has been read");
      return;
    }
    setBusy(true);
    try {
      if (info) {
        const chosen = info.qualities[quality];
        const sound = audio != null ? info.audio[audio] : undefined;
        await api.addMedia({
          url: url.trim(),
          title: name.trim() || info.title,
          folder,
          quality: chosen && { height: chosen.height ?? null, bandwidth: chosen.bandwidth },
          audio: sound && { name: sound.name, language: sound.language ?? null },
          queue: inQueue,
        });
        onClose();
        return;
      }
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
        {media?.status === "reading" && (
          <p role="status" className="text-[13px] text-neutral-500 dark:text-neutral-400">
            Reading the video… (a YouTube video can take a few seconds)
          </p>
        )}
        {media?.status === "failed" && (
          <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
            {media.error}
          </p>
        )}
        {info && (
          <MediaPicker info={info} quality={quality} audio={audio} onQuality={chooseQuality} onAudio={setAudio} />
        )}
        <div className="flex gap-3">
          <div className="min-w-0 flex-1">
            <Field label="File name" htmlFor="add-name">
              <input
                id="add-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={stream ? (info?.title ?? "The video's name") : "The server's name"}
                spellCheck={false}
                className={inputClass}
              />
            </Field>
          </div>
          {!stream && (
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
          )}
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
                disabled={busy || (stream && !info)}
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
          <Button type="submit" primary disabled={busy || (stream && !info)}>
            Download now
          </Button>
        </div>
      </form>
    </dialog>
  );
}
