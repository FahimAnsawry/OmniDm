// SPDX-License-Identifier: GPL-3.0-or-later

// Videos playing on a page (plan §9.2, media detection): HLS and DASH playlists seen in
// the tab's network traffic, read once to tell a whole video from one of its qualities,
// and to spot DRM and live streams, which can't be downloaded.

import type { MediaQuality } from '../../protocol/ts/MediaQuality';

/** `youtube`: a YouTube video, which the app reads with yt-dlp. */
export type StreamKind = 'hls' | 'dash' | 'youtube';

/** The kinds that come as a playlist. */
export type PlaylistKind = Exclude<StreamKind, 'youtube'>;

export type MediaState =
  /** Being read. */
  | 'checking'
  | 'ready'
  /** Locked with DRM. */
  | 'protected'
  /** A live broadcast. */
  | 'live';

export interface FoundMedia {
  url: string;
  kind: StreamKind;
  state: MediaState;
  /** The playlists of its qualities and sound tracks (HLS master playlists). */
  parts?: string[];
  /** One quality or sound track of another video in the list: not shown on its own. */
  part?: boolean;
  /** The frame that asked for it, where its video plays (0: the page itself). */
  frameId?: number;
}

/** What reading a playlist told. */
export interface Inspection {
  /** `undefined`: not a video playlist after all (an internet radio `.m3u`, say). */
  state: Exclude<MediaState, 'checking'> | undefined;
  parts: string[];
}

/** Most videos remembered per tab. */
export const MAX_PER_TAB = 50;

const HLS_TYPES = [
  'application/vnd.apple.mpegurl',
  'application/x-mpegurl',
  'audio/mpegurl',
  'audio/x-mpegurl',
];
const DASH_TYPES = ['application/dash+xml'];

/** Whether a response is a video playlist, by its type or else its name. */
export function streamKind(url: string, contentType?: string): PlaylistKind | undefined {
  const type = contentType?.split(';')[0]?.trim().toLowerCase() ?? '';
  if (HLS_TYPES.includes(type)) return 'hls';
  if (DASH_TYPES.includes(type)) return 'dash';
  let path: string;
  try {
    path = new URL(url).pathname.toLowerCase();
  } catch {
    return undefined;
  }
  if (path.endsWith('.m3u8')) return 'hls';
  if (path.endsWith('.mpd')) return 'dash';
  return undefined;
}

/** A YouTube video's address as `https://www.youtube.com/watch?v=<id>`, for any link to
 * one: a watch page, a short, an embedded player or a `youtu.be` link. */
export function youtubeVideo(url: string): string | undefined {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return undefined;
  }
  if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return undefined;
  const host = parsed.hostname.toLowerCase().replace(/^(www|m|music)\./, '');
  const [first, second] = parsed.pathname.split('/').filter(Boolean);
  let id: string | null | undefined;
  if (host === 'youtu.be') id = first;
  else if (host === 'youtube.com' || host === 'youtube-nocookie.com') {
    if (first === 'watch') id = parsed.searchParams.get('v');
    else if (first && ['shorts', 'embed', 'live', 'v'].includes(first)) id = second;
  }
  return id && /^[A-Za-z0-9_-]{11}$/.test(id) ? `https://www.youtube.com/watch?v=${id}` : undefined;
}

/** A YouTube video found in a frame: ready at once (the app reads it when asked). */
export function youtubeFound(url: string, frameId = 0): FoundMedia {
  return { url, kind: 'youtube', state: 'ready', frameId };
}

/** Reads a playlist's text, fetched from `url`. */
export function inspectPlaylist(kind: PlaylistKind, text: string, url: string): Inspection {
  return kind === 'hls' ? inspectHls(text, url) : inspectDash(text);
}

function inspectHls(text: string, url: string): Inspection {
  const none: Inspection = { state: undefined, parts: [] };
  if (!text.trimStart().startsWith('#EXTM3U')) return none;
  const lines = text.split(/\r?\n/).map((line) => line.trim());
  const master = lines.some((line) => line.startsWith('#EXT-X-STREAM-INF'));
  if (!master && !lines.some((line) => line.startsWith('#EXT-X-TARGETDURATION'))) return none;

  const locked = lines.some((line) => {
    if (!line.startsWith('#EXT-X-KEY:') && !line.startsWith('#EXT-X-SESSION-KEY:')) return false;
    const method = attribute(line, 'METHOD');
    const format = attribute(line, 'KEYFORMAT');
    if (!method || method === 'NONE') return false;
    return !(method === 'AES-128' && (format === undefined || format === 'identity'));
  });

  const parts: string[] = [];
  const add = (reference: string | undefined) => {
    const resolved = resolve(reference, url);
    if (resolved) parts.push(resolved);
  };
  if (master) {
    lines.forEach((line, index) => {
      if (line.startsWith('#EXT-X-STREAM-INF')) {
        add(lines.slice(index + 1).find((next) => next !== '' && !next.startsWith('#')));
      } else if (line.startsWith('#EXT-X-MEDIA:') || line.startsWith('#EXT-X-I-FRAME-STREAM-INF:')) {
        add(attribute(line, 'URI'));
      }
    });
  }

  const vod = lines.some((line) => /^#EXT-X-PLAYLIST-TYPE:\s*VOD/.test(line));
  const live = !master && !lines.includes('#EXT-X-ENDLIST') && !vod;
  return { state: locked ? 'protected' : live ? 'live' : 'ready', parts };
}

function inspectDash(text: string): Inspection {
  if (!/<(\w+:)?MPD[\s>]/.test(text)) return { state: undefined, parts: [] };
  if (/<(\w+:)?ContentProtection[\s>/]/i.test(text)) return { state: 'protected', parts: [] };
  const opening = /<(\w+:)?MPD[^>]*>/.exec(text)?.[0] ?? '';
  if (/\stype\s*=\s*["']dynamic["']/.test(opening)) return { state: 'live', parts: [] };
  return { state: 'ready', parts: [] };
}

/** An attribute of an HLS tag: `NAME=value` or `NAME="value"`. */
function attribute(line: string, name: string): string | undefined {
  const match = new RegExp(`[:,]${name}=("([^"]*)"|[^,]*)`).exec(line);
  if (!match) return undefined;
  return match[2] ?? match[1];
}

function resolve(reference: string | undefined, base: string): string | undefined {
  if (!reference) return undefined;
  try {
    return new URL(reference, base).href;
  } catch {
    return undefined;
  }
}

/** `list` with a newly seen playlist, unless it is there already. */
export function addFound(list: FoundMedia[], url: string, kind: StreamKind, frameId = 0): FoundMedia[] {
  if (list.some((item) => item.url === url)) return list;
  const part = list.some((item) => item.parts?.includes(url));
  const added = [...list, { url, kind, state: 'checking' as const, part, frameId }];
  return added.slice(-MAX_PER_TAB);
}

/** `list` once the playlist at `url` was read: dropped if it isn't a video, and the
 * playlists of its qualities marked as parts of it. */
export function applyInspection(list: FoundMedia[], url: string, inspection: Inspection): FoundMedia[] {
  if (inspection.state === undefined) return list.filter((item) => item.url !== url);
  const parts = new Set(inspection.parts);
  return list.map((item) => {
    if (item.url === url) return { ...item, state: inspection.state!, parts: inspection.parts };
    if (parts.has(item.url)) return { ...item, part: true };
    return item;
  });
}

/** The videos to show: not the qualities of another one. */
export function shown(list: FoundMedia[]): FoundMedia[] {
  return list.filter((item) => !item.part);
}

/** How many shown videos can be downloaded. */
export function downloadable(list: FoundMedia[]): number {
  return shown(list).filter((item) => item.state === 'ready').length;
}

/** The videos to offer in a frame's menu: the ones found there, else the whole tab's. */
export function forFrame(list: FoundMedia[], frameId: number): FoundMedia[] {
  const ready = shown(list).filter((item) => item.state === 'ready');
  const here = ready.filter((item) => (item.frameId ?? 0) === frameId);
  return here.length > 0 ? here : ready;
}

/** "1080p", "1080p60", or "Sound only" for a stream without a picture. */
export function qualityLabel(quality: MediaQuality): string {
  if (quality.height == null && quality.width == null) return 'Sound only';
  const fps = quality.frameRate && quality.frameRate > 30.5 ? Math.round(quality.frameRate) : '';
  return `${quality.height ?? quality.width}p${fps}`;
}

/** "~470 MB", "~1.2 GB". */
export function sizeLabel(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const shown = value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value).toString();
  return `~${shown} ${units[unit]}`;
}

/** A short name for a video: its playlist's file name, or its folder's when generic. */
export function mediaName(url: string): string {
  if (youtubeVideo(url)) return 'YouTube video';
  try {
    const parsed = new URL(url);
    const segments = parsed.pathname.split('/').filter(Boolean).map(decodeURIComponent);
    const last = segments.at(-1) ?? parsed.hostname;
    const stem = last.replace(/\.(m3u8|mpd)$/i, '');
    const generic = /^(master|index|playlist|manifest|chunklist|video|stream|main)$/i.test(stem);
    return generic && segments.length > 1 ? `${segments.at(-2)}/${last}` : last;
  } catch {
    return url;
  }
}
