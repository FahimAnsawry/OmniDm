// SPDX-License-Identifier: GPL-3.0-or-later

// Choosing a video's quality and sound track. A quality already picked (on the video's
// own menu in the browser) is shown, not asked again.

import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
import type { MediaQuality } from "../../../protocol/ts/MediaQuality";
import { formatBytes, formatDuration } from "../lib/format";
import { audioName, formatBitrate, kindName, qualityName } from "../lib/media";
import { selectClass } from "./Fields";

export function MediaPicker({
  info,
  quality,
  audio,
  onQuality,
  onAudio,
  picked = false,
}: {
  info: MediaInfo;
  /** Place in `info.qualities`. */
  quality: number;
  /** Place in `info.audio`, if the quality has separate sound. */
  audio: number | undefined;
  onQuality: (index: number) => void;
  onAudio: (index: number) => void;
  /** The quality was picked already: show it instead of the list. */
  picked?: boolean;
}) {
  const chosen = info.qualities[quality];
  const sounds = chosen?.audio ?? [];
  return (
    <div className="space-y-2.5">
      <div className="flex items-baseline justify-between text-xs font-medium text-neutral-600 dark:text-neutral-400">
        <span id="media-quality">Quality</span>
        <span className="font-normal">
          {kindName(info.kind)}
          {info.duration != null && ` · ${formatDuration(info.duration)}`}
        </span>
      </div>
      {picked && chosen ? (
        <div
          aria-labelledby="media-quality"
          className="flex items-center gap-3 rounded-md border border-neutral-300 bg-sky-50 px-3 py-1.5 text-[13px] dark:border-neutral-700 dark:bg-sky-950/40"
        >
          <QualityCells quality={chosen} />
        </div>
      ) : (
        <div
          role="radiogroup"
          aria-labelledby="media-quality"
          className="max-h-44 overflow-y-auto rounded-md border border-neutral-300 dark:border-neutral-700"
        >
          {info.qualities.map((q, index) => (
            <label
              key={index}
              className={`flex cursor-pointer items-center gap-3 border-b border-neutral-200 px-3 py-1.5 text-[13px] last:border-b-0 dark:border-neutral-800 ${
                index === quality ? "bg-sky-50 dark:bg-sky-950/40" : "hover:bg-neutral-50 dark:hover:bg-neutral-800/50"
              }`}
            >
              <input
                type="radio"
                name="media-quality"
                checked={index === quality}
                onChange={() => onQuality(index)}
                className="accent-sky-600"
              />
              <QualityCells quality={q} />
            </label>
          ))}
        </div>
      )}
      {sounds.length > 1 && (
        <div className="flex items-center gap-3">
          <label htmlFor="media-audio" className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
            Sound
          </label>
          <select
            id="media-audio"
            value={audio ?? sounds[0]}
            onChange={(e) => onAudio(Number(e.target.value))}
            className={`${selectClass} min-w-40`}
          >
            {sounds.map((index) => (
              <option key={index} value={index}>
                {audioName(info.audio[index])}
              </option>
            ))}
          </select>
        </div>
      )}
      {sounds.length > 0 && (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          This video keeps picture and sound apart, so they are saved as two files.
        </p>
      )}
    </div>
  );
}

/** One quality's name, resolution, bitrate and size, in columns. */
function QualityCells({ quality: q }: { quality: MediaQuality }) {
  return (
    <>
      <span className="w-20 font-medium tabular-nums">{qualityName(q)}</span>
      <span className="flex-1 text-neutral-500 tabular-nums dark:text-neutral-400">
        {q.width != null && q.height != null && `${q.width}×${q.height}`}
      </span>
      <span className="text-neutral-500 tabular-nums dark:text-neutral-400">
        {q.bandwidth > 0 && formatBitrate(q.bandwidth)}
      </span>
      <span className="w-20 text-right tabular-nums">{q.size != null && `~${formatBytes(q.size)}`}</span>
    </>
  );
}
