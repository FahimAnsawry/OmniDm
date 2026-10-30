// SPDX-License-Identifier: GPL-3.0-or-later

// Add download: a link (taken from the clipboard when one was copied), where to save it,
// and whether to start now or later in a queue. A video's playlist link (HLS or DASH) is
// read first, to choose its quality.

import { useEffect, useRef, useState } from "react";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { closeDialog } from "../lib/closeDialog";
import { looksLikeStream } from "../lib/media";
import { FileIcon } from "./Chrome";
import { Button, Field, FolderField, inputBase } from "./Fields";
import { Icon } from "./Icon";
import { MediaPicker } from "./MediaPicker";

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
  const [queue, setQueue] = useState(queues[0]?.name ?? "");
  /** Adding: the link is being checked, started by "Download now" or "Later". */
  const [busy, setBusy] = useState<"now" | "later" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stream = looksLikeStream(url);
  const [media, setMedia] = useState<MediaState | null>(null);
  const [quality, setQuality] = useState(0);
  const [audio, setAudio] = useState<number | undefined>();
  const info = media?.status === "ready" ? media.info : undefined;
  /** Plays the dialog's leaving, then closes it. */
  const close = () => closeDialog(dialog.current, onClose);

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
    if (busy) return;
    if (!isWebLink(url)) {
      setError("Enter a link that starts with http:// or https://");
      return;
    }
    if (stream && !info) {
      setError(media?.status === "failed" ? media.error : "Wait until the video has been read");
      return;
    }
    setBusy(inQueue ? "later" : "now");
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
        close();
        return;
      }
      await api.addDownload({
        url,
        fileName: name.trim() || undefined,
        folder,
        queue: inQueue,
      });
      close();
    } catch (e) {
      setError(errorText(e));
      setBusy(null);
    }
  };

  return (
    <dialog
      ref={dialog}
      aria-labelledby="add-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      className="m-auto w-[560px] max-w-[calc(100vw-2rem)] rounded-2xl border border-white/[0.12] bg-[#12172a] p-0 font-sans text-[#f4f6fb] shadow-[0_32px_64px_-12px_rgb(0_0_0/0.75)] backdrop:bg-[#04060c]/70"
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
        className="flex flex-col gap-4 p-5"
      >
        <div className="flex items-start gap-3">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-[#6b88ff]/25 bg-[#4a6cff]/15 text-[#8ea4ff]">
            <Icon name="link" className="size-[18px]" />
          </span>
          <div className="min-w-0 flex-1">
            <h2 id="add-title" className="text-base font-semibold">
              Add download
            </h2>
            <p className="mt-0.5 text-xs text-[#8b93a9]">Paste a link to a file, or to a page with a video.</p>
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={close}
            className="group flex size-8 items-center justify-center rounded-lg text-[#ff8a8f] hover:bg-[#e5484d]/[0.16] hover:text-[#ffc9cb]"
          >
            <Icon name="close" className="size-4 transition-[rotate] duration-300 ease-out motion-safe:group-hover:rotate-90" />
          </button>
        </div>
        <Field label="Link" htmlFor="add-url">
          <div className="relative">
            <Icon name="link" className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-[#8ea4ff]" />
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
              className={`h-10 pl-9 ${media?.status === "reading" ? "pr-9" : "pr-3"} ${inputBase}`}
            />
            {media?.status === "reading" && (
              <Icon
                name="loader"
                className="pointer-events-none absolute top-1/2 right-3 size-4 -translate-y-1/2 text-[#8ea4ff] motion-safe:animate-spin"
              />
            )}
          </div>
        </Field>
        {media?.status === "reading" && <ReadingVideo />}
        {media?.status === "failed" && (
          <p role="alert" className="-mt-1 text-[13px] text-[#ff9a9e]">
            {media.error}
          </p>
        )}
        {info && (
          <MediaPicker info={info} quality={quality} audio={audio} onQuality={chooseQuality} onAudio={setAudio} />
        )}
        <Field label="File name" htmlFor="add-name">
          <div className="relative">
            {(name || url) && (
              <FileIcon
                name={name || (stream ? "video.mp4" : url.split(/[?#]/)[0])}
                flat
                className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2"
              />
            )}
            <input
              id="add-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={stream ? (info?.title ?? "The video's name") : "The server's name"}
              spellCheck={false}
              className={`h-10 pr-3 ${name || url ? "pl-10" : "pl-3"} ${inputBase}`}
            />
          </div>
        </Field>
        <Field
          label={
            <>
              Save in{category && <span className="text-[#8ea4ff]"> · {category}</span>}
            </>
          }
          htmlFor="add-folder"
        >
          <FolderField
            id="add-folder"
            value={folder}
            onChange={(f) => {
              setFolder(f);
              setFolderChosen(true);
            }}
            tall
          />
        </Field>
        {error && (
          <p role="alert" className="text-[13px] text-[#ff9a9e]">
            {error}
          </p>
        )}
        <div className="flex items-center gap-2 border-t border-white/[0.06] pt-4">
          <Button ghost danger onClick={close} className="mr-auto -ml-2">
            Cancel
          </Button>
          {queues.length > 0 && (
            <div className="flex h-9 items-center rounded-[10px] border border-white/[0.12] bg-white/[0.05]">
              <button
                type="button"
                onClick={() => add(queue)}
                disabled={busy != null || (stream && !info)}
                title={`Add to ${queue} without starting`}
                className="flex h-full items-center gap-1.5 rounded-l-[10px] px-3 text-[13px] font-semibold hover:bg-white/[0.06] disabled:pointer-events-none disabled:opacity-40"
              >
                <Icon
                  name={busy === "later" ? "loader" : "clock"}
                  className={`size-3.5 text-[#c9cfdd] ${busy === "later" ? "motion-safe:animate-spin" : ""}`}
                />
                Later
              </button>
              <span className="h-5 w-px bg-white/[0.12]" />
              <select
                aria-label="Queue"
                value={queue}
                onChange={(e) => setQueue(e.target.value)}
                className="select-chevron h-full max-w-36 cursor-pointer appearance-none rounded-r-[10px] bg-transparent pr-8 pl-2.5 text-[12.5px] font-medium text-[#c9cfdd] hover:bg-white/[0.06] focus:outline-none"
              >
                {queues.map((q) => (
                  <option key={q.queueId} value={q.name}>
                    {q.name}
                  </option>
                ))}
              </select>
            </div>
          )}
          {busy === "now" ? (
            // Stays lit while the link is checked, so it reads as working, not unavailable.
            <Button type="submit" primary aria-disabled className="pointer-events-none">
              <Icon name="loader" className="size-3.5 motion-safe:animate-spin" />
              Checking the link…
            </Button>
          ) : (
            <Button type="submit" primary disabled={busy != null || (stream && !info)} className="pr-1.5">
              Download now
              <span className="flex items-center rounded-md bg-white/[0.18] px-1.5 py-0.5">
                <Icon name="enter" className="size-3.5" />
              </span>
            </Button>
          )}
        </div>
      </form>
    </dialog>
  );
}

/** While a video link is read: a spinner and the outline of the quality list to come. */
function ReadingVideo() {
  return (
    <div role="status" className="-mt-1 space-y-2.5">
      <p className="flex items-center gap-2 text-xs font-medium text-[#c9cfdd]">
        <Icon name="loader" className="size-3.5 text-[#8ea4ff] motion-safe:animate-spin" />
        Reading the video…
        <span className="font-normal text-[#8b93a9]">a YouTube video can take a few seconds</span>
      </p>
      <div aria-hidden="true" className="space-y-0.5 rounded-[10px] border border-white/[0.08] bg-white/[0.02] p-1">
        {[0.5, 0.42, 0.34].map((width, i) => (
          <div key={i} className="flex h-[34px] items-center gap-3 px-2.5">
            <span className="size-3.5 rounded-full bg-white/[0.08]" />
            <span className="h-2.5 w-16 rounded bg-white/[0.08] motion-safe:animate-pulse" />
            <span
              className="h-2.5 rounded bg-white/[0.06] motion-safe:animate-pulse"
              style={{ width: `${width * 100}%` }}
            />
            <span className="ml-auto h-2.5 w-12 rounded bg-white/[0.06] motion-safe:animate-pulse" />
          </div>
        ))}
      </div>
    </div>
  );
}
