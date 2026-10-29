// SPDX-License-Identifier: GPL-3.0-or-later

// The video window: its own window, opened when the extension sends a video found on a
// page. It asks for the quality, name and folder, then shows the download's progress.
// It looks like the capture prompt.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api, errorText } from "../lib/api";
import { formatBytes, formatElapsed } from "../lib/format";
import { audioName, kindName, qualityIndex, qualityName } from "../lib/media";
import { Chip, Crumbs, EnterHint, FileIcon, SmallTitleBar, folderCrumbs } from "./Chrome";
import { DownloadProgress } from "./DownloadProgress";
import { Icon, type IconName } from "./Icon";

const close = () => getCurrentWindow().close();

export function MediaWindow({ offer }: { offer: number }) {
  const [url, setUrl] = useState("");
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [quality, setQuality] = useState(0);
  const [audio, setAudio] = useState<number | undefined>();
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [folderChosen, setFolderChosen] = useState(false);
  const [downloadsFolder, setDownloadsFolder] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [laterMenu, setLaterMenu] = useState(false);
  const nameInput = useRef<HTMLInputElement>(null);
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
        setAudio(offered.info.qualities[picked]?.defaultAudio);
        setQueues(queueList);
        const q = offered.info.qualities[picked];
        const ext = q && q.height == null && q.width == null ? "m4a" : "mp4";
        const place = await api.suggestPlace(offered.url, `${offered.info.title}.${ext}`);
        setFolder((current) => current || place.folder);
      })
      .catch((e) => setError(errorText(e)));
  }, [offer]);

  // Switch between Video and Music category folders when toggling video vs sound-only quality.
  const isSoundOnly = Boolean(
    info?.qualities[quality] && info.qualities[quality].height == null && info.qualities[quality].width == null,
  );
  useEffect(() => {
    if (!info || !url || folderChosen) return;
    const ext = isSoundOnly ? "m4a" : "mp4";
    api
      .suggestPlace(url, `${info.title}.${ext}`)
      .then((place) => setFolder(place.folder))
      .catch(() => {});
  }, [info, url, isSoundOnly, folderChosen]);

  useEffect(() => {
    api
      .getSettings()
      .then((settings) => setDownloadsFolder(settings.downloadsFolder))
      .catch(() => {});
  }, []);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (laterMenu) setLaterMenu(false);
        else close();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [laterMenu]);

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: folder || undefined, title: "Save in" });
    if (typeof picked === "string") {
      setFolder(picked);
      setFolderChosen(true);
    }
  };

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
    setLaterMenu(false);
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
    return (
      <DownloadProgress
        id={started}
        name={name.trim() || info.title}
        iconName={isSoundOnly ? "audio.m4a" : "video.mp4"}
        size={info.qualities[quality]?.size}
        folder={folder}
        downloadsFolder={downloadsFolder}
      />
    );
  }

  const chosen = info?.qualities[quality];
  const sounds = chosen?.audio ?? [];
  const tag = heightTag(chosen?.height);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        download();
      }}
      className="glow-window flex h-screen flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none"
    >
      <SmallTitleBar />

      <div className="mt-5 flex flex-col gap-[18px] px-1">
        <div className="flex items-center gap-4">
          <FileIcon name={isSoundOnly ? "audio.m4a" : "video.mp4"} className="h-[66px] w-14" />
          <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
            <div className="flex max-w-full items-center gap-2">
              <input
                ref={nameInput}
                aria-label="File name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                spellCheck={false}
                placeholder="Reading the video…"
                className="-mx-1.5 max-w-full min-w-10 truncate rounded-md border border-transparent bg-transparent px-1.5 text-base font-semibold tracking-[-0.01em] [field-sizing:content] outline-none placeholder:text-[#8b93a9] hover:border-white/10 focus:border-[#6b88ff80] focus:bg-white/[0.04]"
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
            {info && (
              <div className="flex gap-1.5">
                {chosen?.size != null && <Chip>~{formatBytes(chosen.size)}</Chip>}
                <Chip>{kindName(info.kind)}</Chip>
                {info.duration != null && <Chip>{formatElapsed(info.duration)}</Chip>}
              </div>
            )}
            <p className="flex max-w-full items-center gap-1.5 text-xs text-[#8b93a9]" title={url}>
              <Icon name="link-2" className="size-3 text-[#6b7390]" />
              <span className="truncate select-text">{url ? source(url) : "Reading the video…"}</span>
            </p>
          </div>
        </div>

        <div className="rounded-xl border border-white/[0.08] bg-white/[0.03]">
          <Row icon="folder" label="Save to">
            <span className="min-w-0 flex-1" title={folder}>
              <Crumbs crumbs={folderCrumbs(folder, downloadsFolder)} />
            </span>
            <button
              type="button"
              onClick={browse}
              className="rounded-lg border border-white/[0.12] bg-white/[0.06] px-2.5 py-1.5 text-xs font-medium hover:bg-white/[0.1]"
            >
              Change
            </button>
          </Row>
          {info && info.qualities.length > 0 && (
            <>
              <div className="h-px bg-white/[0.06]" />
              <Row icon="video" label="Quality" htmlFor="quality">
                <Select id="quality" icon="clapperboard" value={quality} onChange={(v) => chooseQuality(Number(v))}>
                  {info.qualities.map((q, index) => (
                    <option key={index} value={index}>
                      {qualityName(q)}
                      {q.size != null ? ` · ~${formatBytes(q.size)}` : ""}
                    </option>
                  ))}
                </Select>
                {tag && (
                  <span className="rounded border border-[#e9c46a99] px-1.5 text-xs font-bold tracking-[0.04em] text-[#f2d28b]">
                    {tag}
                  </span>
                )}
              </Row>
            </>
          )}
          {info && sounds.length > 1 && (
            <>
              <div className="h-px bg-white/[0.06]" />
              <Row icon="music" label="Sound" htmlFor="sound">
                <Select id="sound" icon="music" value={audio ?? ""} onChange={(v) => setAudio(Number(v))}>
                  {sounds.map((index) => (
                    <option key={index} value={index}>
                      {audioName(info.audio[index])}
                    </option>
                  ))}
                </Select>
              </Row>
            </>
          )}
        </div>
      </div>

      {error && (
        <p role="alert" className="mt-3 truncate px-1 text-xs text-[#ff9a9e]" title={error}>
          {error}
        </p>
      )}

      <div className="mt-auto flex items-center justify-end gap-2">
        {queues.length > 0 && (
          <div className="relative">
            <button
              type="button"
              aria-haspopup="menu"
              aria-expanded={laterMenu}
              disabled={busy || !info}
              onClick={() => setLaterMenu((shown) => !shown)}
              className="flex h-9 items-center gap-1.5 rounded-[10px] border border-white/[0.12] bg-white/[0.04] pr-2.5 pl-3 text-[13px] font-semibold hover:bg-white/[0.08] disabled:opacity-40"
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
                      onClick={() => download(q.name)}
                      className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] hover:bg-white/[0.07]"
                    >
                      <Icon name={q.schedule ? "moon" : "queue"} className="text-[#8b93a9]" />
                      <span className="flex-1">Add to {q.name}</span>
                      {q.nextStart && (
                        <span className="text-xs text-[#8b93a9] tabular-nums">{q.nextStart.slice(-5)}</span>
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
          disabled={busy || !info}
          className="button-primary flex h-9 items-center gap-3 rounded-[10px] pr-2 pl-4 text-[13px] font-semibold disabled:opacity-50"
        >
          Download now
          <EnterHint />
        </button>
      </div>
    </form>
  );
}

/** "videos.example.com/watch/": where the video comes from. */
function source(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.host + parsed.pathname.replace(/[^/]*$/, "");
  } catch {
    return url;
  }
}

/** "4K", "Full HD"…: a name for a picture height, if it has one. */
function heightTag(height: number | undefined): string | undefined {
  if (height == null) return undefined;
  if (height >= 2160) return "4K";
  if (height >= 1440) return "2K";
  if (height >= 1080) return "Full HD";
  if (height >= 720) return "HD";
  return undefined;
}

/** A line of the options panel: an icon, a label, and its controls. */
function Row({
  icon,
  label,
  htmlFor,
  children,
}: {
  icon: IconName;
  label: string;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex h-[46px] items-center gap-3 px-3">
      <Icon name={icon} className="text-[#8ea4ff]" />
      <label htmlFor={htmlFor} className="w-16 shrink-0 text-[13px] text-[#8b93a9]">
        {label}
      </label>
      {children}
    </div>
  );
}

function Select({
  id,
  icon,
  value,
  onChange,
  children,
}: {
  id: string;
  icon: IconName;
  value: string | number;
  onChange: (value: string) => void;
  children: ReactNode;
}) {
  return (
    <div className="relative min-w-0">
      <Icon
        name={icon}
        className="pointer-events-none absolute top-1/2 left-2.5 size-3 -translate-y-1/2 text-[#c9cfdd]"
      />
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="max-w-72 appearance-none truncate rounded-lg border border-white/[0.12] bg-[#161c30] py-1.5 pr-7 pl-7 text-xs font-medium text-[#f4f6fb] outline-none hover:bg-[#1d2540] focus:border-[#6b88ff80]"
      >
        {children}
      </select>
      <Icon
        name="up-down"
        className="pointer-events-none absolute top-1/2 right-2 size-3 -translate-y-1/2 text-[#8b93a9]"
      />
    </div>
  );
}
