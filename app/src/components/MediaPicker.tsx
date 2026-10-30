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
      <div className="flex items-baseline justify-between text-xs font-medium text-[#c9cfdd]">
        <span id="media-quality">Quality</span>
        <span className="font-normal text-[#8b93a9]">
          {kindName(info.kind)}
          {info.duration != null && ` · ${formatDuration(info.duration)}`}
        </span>
      </div>
      {picked && chosen ? (
        <div
          aria-labelledby="media-quality"
          className="flex items-center gap-3 rounded-[10px] border border-[#6b88ff]/30 bg-[#4a6cff]/[0.12] px-3 py-2 text-[13px]"
        >
          <QualityCells quality={chosen} />
        </div>
      ) : (
        <div
          role="radiogroup"
          aria-labelledby="media-quality"
          className="max-h-44 space-y-0.5 overflow-y-auto rounded-[10px] border border-white/[0.08] bg-white/[0.02] p-1"
        >
          {info.qualities.map((q, index) => (
            <label
              key={index}
              className={`flex cursor-pointer items-center gap-3 rounded-lg border px-2.5 py-1.5 text-[13px] ${
                index === quality ? "border-[#6b88ff]/30 bg-[#4a6cff]/[0.12]" : "border-transparent hover:bg-white/[0.04]"
              }`}
            >
              <input
                type="radio"
                name="media-quality"
                checked={index === quality}
                onChange={() => onQuality(index)}
                className="accent-[#4a6cff]"
              />
              <QualityCells quality={q} />
            </label>
          ))}
        </div>
      )}
      {sounds.length > 1 && (
        <div className="flex items-center gap-3">
          <label htmlFor="media-audio" className="text-xs font-medium text-[#c9cfdd]">
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
        <p className="text-xs text-[#8b93a9]">
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
      <span className="flex-1 font-mono text-[#8b93a9] tabular-nums">
        {q.width != null && q.height != null && `${q.width}×${q.height}`}
      </span>
      <span className="font-mono text-[#8b93a9] tabular-nums">
        {q.bandwidth > 0 && formatBitrate(q.bandwidth)}
      </span>
      <span className="w-20 text-right font-mono tabular-nums">{q.size != null && `~${formatBytes(q.size)}`}</span>
    </>
  );
}
