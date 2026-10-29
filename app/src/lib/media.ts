// SPDX-License-Identifier: GPL-3.0-or-later

// Video streams (HLS and DASH) in the Add download dialog.

import type { MediaAudio } from "../../../protocol/ts/MediaAudio";
import type { MediaQuality } from "../../../protocol/ts/MediaQuality";
import type { QualityPick } from "./api";

/** A link to a video's playlist (`.m3u8`) or manifest (`.mpd`), by its name, or to a
 * YouTube video (read with yt-dlp). */
export function looksLikeStream(text: string): boolean {
  try {
    const url = new URL(text.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") return false;
    return /\.(m3u8|m3u|mpd)$/i.test(url.pathname) || isYoutube(url);
  } catch {
    return false;
  }
}

/** A YouTube video's page: a watch page, a short, an embedded player or a youtu.be link. */
function isYoutube(url: URL): boolean {
  const host = url.hostname.toLowerCase().replace(/^(www|m|music)\./, "");
  const [first, second] = url.pathname.split("/").filter(Boolean);
  const id =
    host === "youtu.be"
      ? first
      : host === "youtube.com" || host === "youtube-nocookie.com"
        ? first === "watch"
          ? url.searchParams.get("v")
          : ["shorts", "embed", "live", "v"].includes(first ?? "")
            ? second
            : undefined
        : undefined;
  return !!id && /^[A-Za-z0-9_-]{11}$/.test(id);
}

/** "HLS video", "DASH video", "YouTube video". */
export function kindName(kind: string): string {
  return kind === "youtube" ? "YouTube video" : kind === "dash" ? "DASH video" : "HLS video";
}

/** The place of `pick` in `qualities`: the same height first, then the nearest bandwidth. */
export function qualityIndex(qualities: MediaQuality[], pick: QualityPick | null): number {
  if (!pick) return 0;
  let best = 0;
  let bestScore = Infinity;
  qualities.forEach((q, index) => {
    const score = ((q.height ?? null) === pick.height ? 0 : 1e15) + Math.abs(q.bandwidth - pick.bandwidth);
    if (score < bestScore) {
      best = index;
      bestScore = score;
    }
  });
  return best;
}

/** "2.5 Mbit/s", "128 kbit/s". */
export function formatBitrate(bitsPerSecond: number): string {
  return bitsPerSecond >= 1_000_000
    ? `${(bitsPerSecond / 1_000_000).toFixed(1)} Mbit/s`
    : `${Math.round(bitsPerSecond / 1000)} kbit/s`;
}

/** "1080p", "1080p60", or "Sound only (130 kbit/s)" for a stream without a picture. */
export function qualityName(quality: MediaQuality): string {
  if (quality.height == null && quality.width == null) {
    return quality.bandwidth > 0 ? `Sound only (${formatBitrate(quality.bandwidth)})` : "Sound only";
  }
  const lines = quality.height ?? quality.width;
  const fps = quality.frameRate && quality.frameRate > 30.5 ? Math.round(quality.frameRate) : "";
  return `${lines}p${fps}`;
}

/** "English", "Deutsch (de)". */
export function audioName(audio: MediaAudio): string {
  const { name, language } = audio;
  if (!language || name.toLowerCase().includes(language.toLowerCase())) return name;
  return `${name} (${language})`;
}
