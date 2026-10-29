// SPDX-License-Identifier: GPL-3.0-or-later

// The video window: its own window, opened when the extension sends a video found on a
// page. It asks for the quality, name and folder, then shows the download's progress.

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { qualityIndex } from "../lib/media";
import { DownloadProgress } from "./DownloadProgress";
import { Button, Field, FolderField, inputClass, selectClass } from "./Fields";
import { Icon } from "./Icon";
import { MediaPicker } from "./MediaPicker";

const close = () => getCurrentWindow().close();

export function MediaWindow({ offer }: { offer: number }) {
  const [url, setUrl] = useState("");
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [quality, setQuality] = useState(0);
  /** The quality was picked on the video's menu in the browser: it isn't asked again. */
  const [picked, setPicked] = useState(false);
  const [audio, setAudio] = useState<number | undefined>();
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [queue, setQueue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Started with "Download now": the window shows its progress from then on. */
  const [started, setStarted] = useState<number | null>(null);

  useEffect(() => {
    Promise.all([api.mediaOffer(offer), api.listQueues()])
      .then(async ([offered, queueList]) => {
        setUrl(offered.url);
        setInfo(offered.info);
        setName(offered.info.title);
        // The quality picked on the video's menu, if it was.
        const picked = qualityIndex(offered.info.qualities, offered.quality);
        setQuality(picked);
        setPicked(offered.quality != null);
        setAudio(offered.info.qualities[picked]?.defaultAudio);
        setQueues(queueList);
        setQueue(queueList[0]?.name ?? "");
        // Named as an .mp4, so it lands with the videos.
        const place = await api.suggestPlace(offered.url, `${offered.info.title}.mp4`);
        setFolder((current) => current || place.folder);
      })
      .catch((e) => setError(errorText(e)));
  }, [offer]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  const chooseQuality = (index: number) => {
    setQuality(index);
    const sounds = info?.qualities[index]?.audio ?? [];
    setAudio((current) =>
      current != null && sounds.includes(current) ? current : info?.qualities[index]?.defaultAudio,
    );
  };

  const download = async (inQueue?: string) => {
    if (!info) return;
    const chosen = info.qualities[quality];
    const sound = audio != null ? info.audio[audio] : undefined;
    setBusy(true);
    try {
      const id = await api.acceptMedia({
        offer,
        title: name.trim() || info.title,
        folder,
        quality: chosen && { height: chosen.height ?? null, bandwidth: chosen.bandwidth },
        audio: sound && { name: sound.name, language: sound.language ?? null },
        queue: inQueue,
      });
      if (inQueue) {
        await close();
      } else {
        // Now an ordinary window that can go behind others.
        await getCurrentWindow()
          .setAlwaysOnTop(false)
          .catch(() => {});
        setStarted(id);
      }
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  if (started != null && info) {
    return <DownloadProgress id={started} name={name.trim() || info.title} size={info.qualities[quality]?.size} />;
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        download();
      }}
      className="flex h-screen flex-col gap-3.5 bg-white p-5 font-sans text-neutral-900 select-none dark:bg-neutral-900 dark:text-neutral-100"
    >
      <div className="flex items-start gap-3">
        <span className="rounded-lg bg-sky-100 p-2 text-sky-700 dark:bg-sky-900/50 dark:text-sky-300">
          <Icon name="video" className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h1 className="text-[15px] font-semibold">Download this video?</h1>
          <p className="truncate text-xs text-neutral-500 select-text" title={url}>
            {url || "Reading the video…"}
          </p>
        </div>
      </div>

      {info ? (
        <MediaPicker
          info={info}
          quality={quality}
          audio={audio}
          onQuality={chooseQuality}
          onAudio={setAudio}
          picked={picked}
        />
      ) : (
        !error && <p className="text-[13px] text-neutral-500 dark:text-neutral-400">Reading the video…</p>
      )}

      <Field label="File name" htmlFor="name">
        <input
          id="name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          spellCheck={false}
          className={inputClass}
        />
      </Field>
      <Field label="Save in" htmlFor="folder">
        <FolderField id="folder" value={folder} onChange={setFolder} />
      </Field>

      {error && (
        <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center justify-end gap-2">
        <Button onClick={close} className="mr-auto">
          Cancel
        </Button>
        {queues.length > 0 && (
          <div className="flex">
            <Button
              onClick={() => download(queue)}
              disabled={busy || !info}
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
        <Button type="submit" primary autoFocus disabled={busy || !info}>
          Download now
        </Button>
      </div>
    </form>
  );
}
