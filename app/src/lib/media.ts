// SPDX-License-Identifier: GPL-3.0-or-later

// Video streams (HLS and DASH) and video sites' posts in the Add download dialog.

import type { MediaAudio } from "../../../protocol/ts/MediaAudio";
import type { MediaQuality } from "../../../protocol/ts/MediaQuality";
import type { QualityPick } from "./api";

/** A link to a video's playlist (`.m3u8`) or manifest (`.mpd`), by its name, or to one
 * video or post on YouTube, TikTok, Instagram, Facebook or X (read with yt-dlp). */
export function looksLikeStream(text: string): boolean {
  try {
    const url = new URL(text.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") return false;
    return /\.(m3u8|m3u|mpd)$/i.test(url.pathname) || isYoutube(url) || isSitePost(url);
  } catch {
    return false;
  }
}

/** One post on TikTok, Instagram, Facebook or X, by its address. Only the sites' own hosts
 * and their subdomains count (the service checks the same way). */
function isSitePost(url: URL): boolean {
  const host = url.hostname.toLowerCase();
  const on = (domain: string) => host === domain || host.endsWith(`.${domain}`);
  const [a = "", b = "", c = "", d = ""] = url.pathname.split("/").filter(Boolean);
  const code = (s: string) => /^[A-Za-z0-9_-]+$/.test(s);
  const number = (s: string) => /^\d+$/.test(s);
  if (on("vm.tiktok.com") || on("vt.tiktok.com") || on("fb.watch")) return code(a);
  if (on("tiktok.com")) {
    return (a.startsWith("@") && b === "video" && number(c)) || (a === "embed" && number(b)) || (a === "t" && code(b));
  }
  if (on("instagram.com")) {
    const kinds = ["p", "reel", "reels", "tv"];
    return (kinds.includes(a) && code(b)) || (kinds.includes(b) && code(c));
  }
  if (on("x.com") || on("twitter.com")) return code(a) && b === "status" && number(c);
  if (on("facebook.com")) {
    const v = url.searchParams.get("v") ?? "";
    return (
      ((a === "watch" || a === "video.php") && number(v)) ||
      (a === "reel" && number(b)) ||
      (a === "share" && (b === "v" || b === "r") && code(c)) ||
      (b === "videos" && (number(c) || number(d)))
    );
  }
  return false;
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

const KIND_NAMES: Record<string, string> = {
  youtube: "YouTube video",
  tiktok: "TikTok video",
  instagram: "Instagram video",
  facebook: "Facebook video",
  x: "X video",
  dash: "DASH video",
};

/** "HLS video", "DASH video", "YouTube video", "TikTok video"… */
export function kindName(kind: string): string {
  return Object.prototype.hasOwnProperty.call(KIND_NAMES, kind) ? KIND_NAMES[kind] : "HLS video";
}

/** The place of `pick` in `qualities`: the same video of a post first, then the same
 * height, then the nearest bandwidth. */
export function qualityIndex(qualities: MediaQuality[], pick: QualityPick | null): number {
  if (!pick) return 0;
  let best = 0;
  let bestScore = Infinity;
  qualities.forEach((q, index) => {
    const score =
      ((q.video ?? null) === (pick.video ?? null) ? 0 : 2e15) +
      ((q.height ?? null) === pick.height ? 0 : 1e15) +
      Math.abs(q.bandwidth - pick.bandwidth);
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

/** "1080p", "1080p60", or "Sound only (130 kbit/s)" for a stream without a picture; with
 * "Video 2 · " before it in a post holding several videos. */
export function qualityName(quality: MediaQuality): string {
  const video = quality.video != null ? `Video ${quality.video} · ` : "";
  if (quality.height == null && quality.width == null) {
    return video + (quality.bandwidth > 0 ? `Sound only (${formatBitrate(quality.bandwidth)})` : "Sound only");
  }
  const lines = quality.height ?? quality.width;
  const fps = quality.frameRate && quality.frameRate > 30.5 ? Math.round(quality.frameRate) : "";
  return `${video}${lines}p${fps}`;
}

/** "English", "Deutsch (de)". */
export function audioName(audio: MediaAudio): string {
  const { name, language } = audio;
  if (!language || name.toLowerCase().includes(language.toLowerCase())) return name;
  return `${name} (${language})`;
}
