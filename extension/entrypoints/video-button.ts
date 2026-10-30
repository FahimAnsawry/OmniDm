// SPDX-License-Identifier: GPL-3.0-or-later

// The Download button on a video. The background puts this script into a frame only once
// it found a video playlist there. The button shows over the top-right corner of a video
// while the pointer is on it; it opens a menu of the video's qualities, and picking one
// opens OmniDM's video window with that quality chosen.
//
// Everything is drawn in a closed shadow root, so the page's styles can't reach it, and
// text from the network is only ever set as text.

import { browser } from 'wxt/browser';
import type { MediaInfo } from '../../protocol/ts/MediaInfo';
import type { MediaQuality } from '../../protocol/ts/MediaQuality';
import { mediaName, qualityLabel, sizeLabel } from '../lib/media';

/** Smaller videos (previews, ads, avatars) get no button. */
const MIN_WIDTH = 240;
const MIN_HEIGHT = 135;
/** How long the button stays after the pointer leaves the video. */
const HIDE_AFTER_MS = 1500;
/** How long "Opened in OmniDM" shows before the menu closes. */
const DONE_SHOWN_MS = 1500;

/** A video of this frame and what it offers, as the background answers `videoQualities`. */
export type FrameVideo = { url: string; info?: MediaInfo; error?: string };

const CSS = `
:host { all: initial; }
.wrap {
  position: relative; color: #f4f6fb;
  font: 13px/1.25 Inter, "Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, sans-serif;
  -webkit-font-smoothing: antialiased;
}
button { font: inherit; color: inherit; }
.button {
  display: flex; align-items: center; gap: 6px; height: 36px; padding: 6px 8px 6px 6px;
  border: 1px solid rgb(255 255 255 / 0.18); border-radius: 999px; background: rgb(13 17 34 / 0.8);
  -webkit-backdrop-filter: blur(20px); backdrop-filter: blur(20px);
  box-shadow: 0 8px 24px -4px rgb(0 0 0 / 0.35); cursor: pointer; box-sizing: border-box;
}
.button:hover { background: rgb(20 26 48 / 0.88); }
.button:focus-visible { outline: 2px solid #8ea4ff; outline-offset: 2px; }
.button .logo { width: 24px; height: 24px; }
.button .rule { width: 1px; height: 16px; background: rgb(255 255 255 / 0.2); }
.button .chevron { width: 16px; height: 16px; color: rgb(255 255 255 / 0.7); transition: transform 0.15s; transform: rotate(180deg); }
.button[aria-expanded="true"] .chevron { transform: none; }
.button .close {
  display: flex; align-items: center; justify-content: center; width: 20px; height: 20px;
  border-radius: 999px; color: rgb(255 255 255 / 0.55); transition: background 0.15s, color 0.15s;
}
.button .close:hover { background: rgb(255 255 255 / 0.16); color: #fff; }
.button .close svg { width: 13px; height: 13px; }
.menu {
  position: absolute; top: calc(100% + 8px); right: 0; width: 320px; box-sizing: border-box;
  max-height: min(460px, 75vh); overflow-y: auto; padding: 16px 6px 6px;
  border: 1px solid rgb(255 255 255 / 0.14); border-radius: 18px; background: rgb(14 18 36 / 0.85);
  -webkit-backdrop-filter: blur(28px); backdrop-filter: blur(28px);
  box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.08), 0 24px 56px -8px rgb(0 0 0 / 0.6);
  scrollbar-width: thin; scrollbar-color: rgb(255 255 255 / 0.15) transparent;
}
.menu[hidden] { display: none; }
.head { display: flex; align-items: center; gap: 12px; padding: 0 10px; }
.thumb {
  flex: none; width: 64px; height: 40px; border-radius: 8px; background: #1a2036;
  box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.15); object-fit: cover;
}
.titles { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.name { overflow: hidden; font-size: 14px; font-weight: 600; white-space: nowrap; text-overflow: ellipsis; }
.sub { overflow: hidden; color: #a3abc2; font-size: 12px; white-space: nowrap; text-overflow: ellipsis; }
.tabs { display: flex; gap: 0; margin: 12px 0 6px; padding: 3px; border-radius: 10px; background: rgb(255 255 255 / 0.06); }
.tab {
  display: flex; flex: 1; align-items: center; justify-content: center; gap: 6px; height: 28px;
  border: 0; border-radius: 8px; background: none; color: #a3abc2; font-size: 12px; font-weight: 500; cursor: pointer;
}
.tab svg { width: 12px; height: 12px; }
.tab[aria-selected="true"] { background: rgb(255 255 255 / 0.12); color: #fff; font-weight: 600; box-shadow: 0 1px 2px rgb(0 0 0 / 0.25); }
.tab:focus-visible { outline: 2px solid #8ea4ff; }
.group { padding: 10px 10px 4px; color: #a3abc2; font-size: 11px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; }
.item {
  display: flex; width: 100%; height: 44px; box-sizing: border-box; align-items: center; justify-content: space-between;
  gap: 12px; margin-top: 2px; padding: 7px 6px 7px 10px; border: 1px solid transparent; border-radius: 10px;
  background: none; text-align: left; cursor: pointer; outline: none;
}
.quality { display: flex; align-items: center; gap: 12px; min-width: 0; }
.res { font-size: 14px; font-weight: 600; white-space: nowrap; }
.tag {
  padding: 1px 5px 0 6px; border: 1px solid rgb(255 255 255 / 0.2); border-radius: 4px;
  color: #c9cfdd; font-size: 12px; font-weight: 700; letter-spacing: 0.04em; white-space: nowrap;
}
.size { padding-right: 4px; color: #a3abc2; font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.pill {
  display: none; align-items: center; gap: 6px; height: 30px; padding: 0 10px; box-sizing: border-box;
  border: 1px solid rgb(255 255 255 / 0.2); border-radius: 8px; background: linear-gradient(180deg, #4265f7, #2546d6);
  box-shadow: 0 4px 14px -2px rgb(59 91 255 / 0.6); color: #fff; font-size: 12px; font-weight: 600; white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.pill svg { width: 12px; height: 12px; }
.item.active { border-color: rgb(233 196 106 / 0.4); background: rgb(233 196 106 / 0.12); }
.item.active .res { color: #f6dc9c; }
.item.active .tag { border-color: rgb(233 196 106 / 0.6); color: #f2d28b; }
.item.active .size { display: none; }
.item.active .pill { display: flex; }
.note { padding: 12px 10px 8px; color: #c9cfdd; }
.error { padding: 12px 10px 8px; color: #ff9a9e; }
`;

/** The OmniDM mark: a ring broken by a downward chevron, in sunset colours. */
const LOGO =
  '<svg class="logo" viewBox="78 70 356 356" aria-hidden="true"><defs><linearGradient id="s" gradientUnits="userSpaceOnUse" x1="106" y1="76" x2="406" y2="420"><stop offset="0" stop-color="#ff8a4c"/><stop offset="1" stop-color="#ee3f6a"/></linearGradient><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="512" height="512"><rect width="512" height="512" fill="#fff"/><path d="M172 296L256 380L340 296" fill="none" stroke="#000" stroke-width="96" stroke-linejoin="miter"/></mask></defs><circle cx="256" cy="226" r="122" fill="none" stroke="url(#s)" stroke-width="56" mask="url(#m)"/><path d="M172 296L256 380L340 296" fill="none" stroke="url(#s)" stroke-width="56" stroke-linejoin="miter"/></svg>';

const svg = (paths: string, className = '') =>
  `<svg${className ? ` class="${className}"` : ''} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
const CHEVRON = svg('<path d="m18 15-6-6-6 6"/>', 'chevron');
const CLOSE_ICON = svg('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>');
const VIDEO_ICON = svg(
  '<path d="M20.2 6 3 11l-.9-2.4c-.3-1.1.3-2.2 1.3-2.5l13.5-4c1.1-.3 2.2.3 2.5 1.3Z"/><path d="m6.2 5.3 3.1 3.9"/><path d="m12.4 3.4 3.1 4"/><path d="M3 11h18v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
);
const SOUND_ICON = svg('<circle cx="8" cy="18" r="4"/><path d="M12 18V2l7 4"/>');
const DOWNLOAD_ICON = svg('<path d="M12 17V3"/><path d="m6 11 6 6 6-6"/><path d="M19 21H5"/>');

/** "4K", "Full HD"…: a name for a picture height, if it has one. */
function heightTag(height: number | undefined): string | undefined {
  if (height == null) return undefined;
  if (height >= 2160) return '4K';
  if (height >= 1440) return '2K';
  if (height >= 1080) return 'Full HD';
  if (height >= 720) return 'HD';
  return undefined;
}

/** "12:48", "1:02:05". */
function clock(seconds: number): string {
  const s = Math.round(seconds);
  const two = (n: number) => String(n).padStart(2, '0');
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

export default defineUnlistedScript(() => {
  const marked = window as Window & { omnidmVideoButton?: boolean };
  // Put in again after another video was found: one button is enough.
  if (marked.omnidmVideoButton) return;
  marked.omnidmVideoButton = true;
  start();
});

function start() {
  const host = document.createElement('omnidm-video-button');
  host.style.cssText = 'all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; display: none;';
  const root = host.attachShadow({ mode: 'closed' });
  root.innerHTML = `<style>${CSS}</style><div class="wrap"><button class="button" type="button" aria-haspopup="menu" aria-expanded="false" aria-label="Download with OmniDM" title="Download with OmniDM">${LOGO}<span class="rule"></span>${CHEVRON}<span class="close" role="button" tabindex="0" aria-label="Hide button" title="Hide button on this video">${CLOSE_ICON}</span></button><div class="menu" role="menu" hidden></div></div>`;
  const button = root.querySelector<HTMLButtonElement>('.button')!;
  const closeBtn = root.querySelector<HTMLSpanElement>('.close')!;
  const menu = root.querySelector<HTMLDivElement>('.menu')!;
  document.documentElement.append(host);

  // After the extension is reloaded or updated, this copy stays in the page but can't reach
  // it any more ("Extension context invalidated"): it then takes its button away and stops
  // listening, and the new copy puts in its own.
  const stop = new AbortController();
  const { signal } = stop;
  const alive = () => {
    if (browser.runtime?.id) return true;
    stop.abort();
    host.remove();
    return false;
  };
  const send = <T>(message: object): Promise<T> =>
    alive() ? browser.runtime.sendMessage(message) : Promise.reject(new Error('OmniDM was reloaded'));

  let target: HTMLVideoElement | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let menuOpen = false;
  let enabled = true;
  /** Videos dismissed with the × button, keyed by the page + stream URL at the time. */
  let dismissed = new WeakMap<HTMLVideoElement, string>();
  const videoKey = (video: HTMLVideoElement) => `${location.href}|${video.currentSrc}`;
  const isDismissed = (video: HTMLVideoElement) => dismissed.get(video) === videoKey(video);
  /** What the menu shows once read: the videos, and which tab. */
  let videos: FrameVideo[] = [];
  let tab: 'video' | 'sound' = 'video';

  const bigEnough = (rect: DOMRect) => rect.width >= MIN_WIDTH && rect.height >= MIN_HEIGHT;

  /** The video under a point, if it is big enough for a button. */
  const videoAt = (x: number, y: number) => {
    if (!enabled) return undefined;
    for (const video of document.querySelectorAll('video')) {
      if (isDismissed(video)) continue;
      const rect = video.getBoundingClientRect();
      if (bigEnough(rect) && x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
        return video;
      }
    }
    return undefined;
  };

  /** Keeps the button on the video's top-right corner, inside the window. */
  const place = () => {
    if (!target) return;
    const rect = target.getBoundingClientRect();
    if (!target.isConnected || !bigEnough(rect) || rect.bottom < 0 || rect.top > innerHeight) {
      hide(true);
      return;
    }
    host.style.top = `${Math.max(rect.top, 0) + 12}px`;
    host.style.left = `${Math.min(rect.right, innerWidth) - 12}px`;
    host.style.transform = 'translateX(-100%)';
  };

  const show = (video: HTMLVideoElement) => {
    if (!enabled || isDismissed(video)) return;
    clearTimeout(hideTimer);
    if (target !== video) closeMenu();
    if (!target) void send({ type: 'videoQualities' }).catch(() => undefined);
    target = video;
    host.style.display = 'block';
    place();
  };

  const hide = (now = false) => {
    clearTimeout(hideTimer);
    if (menuOpen && !now) return;
    const go = () => {
      closeMenu();
      host.style.display = 'none';
      target = undefined;
    };
    if (now) go();
    else hideTimer = setTimeout(go, HIDE_AFTER_MS);
  };

  const closeMenu = () => {
    menuOpen = false;
    menu.hidden = true;
    button.setAttribute('aria-expanded', 'false');
  };

  const element = <K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string) => {
    const made = document.createElement(tag);
    made.className = className;
    if (text !== undefined) made.textContent = text;
    return made;
  };

  /** A still of the video playing, for the menu's heading. */
  const thumbnail = () => {
    const canvas = element('canvas', 'thumb');
    canvas.width = 128;
    canvas.height = 80;
    const video = target;
    if (video && video.readyState >= 2 && video.videoWidth > 0) {
      const scale = Math.max(canvas.width / video.videoWidth, canvas.height / video.videoHeight);
      const width = video.videoWidth * scale;
      const height = video.videoHeight * scale;
      try {
        canvas
          .getContext('2d')
          ?.drawImage(video, (canvas.width - width) / 2, (canvas.height - height) / 2, width, height);
      } catch {
        // Not drawable: the dark box stays.
      }
    }
    return canvas;
  };

  /** The heading: a still, the video's name, and "HLS · 12:48 · up to 4K". */
  const heading = (video: FrameVideo | undefined) => {
    const head = element('div', 'head');
    const titles = element('div', 'titles');
    const info = video?.info;
    const name = info?.title || (video ? mediaName(video.url) : 'Video');
    titles.append(element('div', 'name', name));
    if (info) {
      const best = info.qualities.find((q) => q.height != null);
      const most = best ? (heightTag(best.height) ?? `${best.height}p`) : undefined;
      const facts = [
        info.kind === 'youtube' ? 'YouTube' : info.kind.toUpperCase(),
        info.duration ? clock(info.duration) : undefined,
        most ? `up to ${most}` : undefined,
      ];
      titles.append(element('div', 'sub', facts.filter(Boolean).join(' · ')));
    }
    titles.title = name;
    head.append(thumbnail(), titles);
    return head;
  };

  /** Shows a line of text under the heading, in place of the qualities. */
  const say = (className: 'note' | 'error', text: string) => {
    menu.replaceChildren(heading(videos[0]), element('div', className, text));
  };

  const choose = async (url: string, quality: { height?: number; bandwidth: number }) => {
    say('note', 'Opening OmniDM…');
    try {
      const reply = await send<{ ok: true } | { error: string }>({ type: 'videoChoose', url, quality });
      if ('error' in reply) {
        say('error', reply.error);
        return;
      }
      say('note', 'Opened in OmniDM');
      setTimeout(() => hide(true), DONE_SHOWN_MS);
    } catch {
      say('error', "OmniDM couldn't be reached.");
    }
  };

  /** Marks one quality as the one Enter or a click downloads. */
  const activate = (item: HTMLElement) => {
    for (const other of menu.querySelectorAll('.item.active')) other.classList.remove('active');
    item.classList.add('active');
  };

  const qualityItem = (video: FrameVideo, quality: MediaQuality) => {
    const item = element('button', 'item');
    item.type = 'button';
    item.setAttribute('role', 'menuitem');
    const left = element('span', 'quality');
    const sound = quality.height == null && quality.width == null;
    left.append(
      element(
        'span',
        'res',
        sound && quality.bandwidth ? `${Math.round(quality.bandwidth / 1000)} kbps` : qualityLabel(quality),
      ),
    );
    const tag = heightTag(quality.height);
    if (tag) left.append(element('span', 'tag', tag));
    const size = quality.size != null ? sizeLabel(quality.size) : '';
    const pill = element('span', 'pill');
    pill.innerHTML = DOWNLOAD_ICON;
    pill.append(size || 'Download');
    item.append(left, element('span', 'size', size), pill);
    item.addEventListener('pointerenter', () => activate(item));
    item.addEventListener('focus', () => activate(item));
    item.addEventListener('click', (event) => {
      event.stopPropagation();
      void choose(video.url, { height: quality.height, bandwidth: quality.bandwidth });
    });
    return item;
  };

  /** Draws the read videos: heading, Video / Sound only tabs, and the qualities. */
  const render = () => {
    const parts: HTMLElement[] = [heading(videos[0])];
    const sound = (q: MediaQuality) => q.height == null && q.width == null;
    const tabs = element('div', 'tabs');
    tabs.setAttribute('role', 'tablist');
    for (const [name, label, icon] of [
      ['video', 'Video', VIDEO_ICON],
      ['sound', 'Sound only', SOUND_ICON],
    ] as const) {
      const button = element('button', 'tab');
      button.type = 'button';
      button.setAttribute('role', 'tab');
      button.setAttribute('aria-selected', String(tab === name));
      button.innerHTML = icon;
      button.append(label);
      button.addEventListener('click', (event) => {
        event.stopPropagation();
        tab = name;
        render();
        menu.querySelector<HTMLButtonElement>('.item')?.focus();
      });
      tabs.append(button);
    }
    parts.push(tabs);
    let any = false;
    videos.forEach((video, index) => {
      // Several videos on one page: each after the first under its own name.
      if (index > 0) parts.push(element('div', 'group', video.info?.title || mediaName(video.url)));
      if (!video.info) {
        parts.push(element('div', 'error', video.error ?? "This video can't be downloaded."));
        return;
      }
      for (const quality of video.info.qualities) {
        if (sound(quality) !== (tab === 'sound')) continue;
        parts.push(qualityItem(video, quality));
        any = true;
      }
    });
    if (!any && videos.some((v) => v.info)) {
      parts.push(
        element(
          'div',
          'note',
          tab === 'sound' ? 'This video has no sound-only version.' : 'Only sound can be downloaded here.',
        ),
      );
    }
    menu.replaceChildren(...parts);
    const first = menu.querySelector<HTMLButtonElement>('.item');
    if (first) activate(first);
  };

  const openMenu = async () => {
    menuOpen = true;
    menu.hidden = false;
    button.setAttribute('aria-expanded', 'true');
    videos = [];
    tab = 'video';
    say('note', 'Reading the qualities…');
    try {
      const reply = await send<{ videos: FrameVideo[] }>({ type: 'videoQualities' });
      videos = reply.videos;
    } catch {
      say('error', "OmniDM couldn't be reached.");
      return;
    }
    if (!menuOpen) return;
    if (videos.length === 0) {
      say('note', 'No video to download here any more.');
      return;
    }
    // A video with only sound opens on that tab.
    const qualities = videos.flatMap((v) => v.info?.qualities ?? []);
    if (qualities.length > 0 && qualities.every((q) => q.height == null && q.width == null)) tab = 'sound';
    render();
    menu.querySelector<HTMLButtonElement>('.item')?.focus();
  };

  // Up and down move between the qualities.
  menu.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const items = [...menu.querySelectorAll<HTMLButtonElement>('.item')];
    if (items.length === 0) return;
    event.preventDefault();
    const at = items.indexOf(root.activeElement as HTMLButtonElement);
    const next = event.key === 'ArrowDown' ? at + 1 : at - 1;
    items[(next + items.length) % items.length]?.focus();
  });

  const dismissCurrent = (event: Event) => {
    event.stopPropagation();
    event.preventDefault();
    if (target) dismissed.set(target, videoKey(target));
    hide(true);
  };
  closeBtn.addEventListener('click', dismissCurrent);
  closeBtn.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') dismissCurrent(event);
  });

  // Respect the extension enabled state and "Show Download button on videos" setting immediately.
  void browser.storage?.local
    ?.get('settings')
    .then((stored) => {
      const settings = stored?.settings as { enabled?: boolean; videoButton?: boolean } | undefined;
      if (settings?.enabled === false || settings?.videoButton === false) {
        enabled = false;
        hide(true);
      }
    })
    .catch(() => undefined);
  const onSettings = (changes: Record<string, { newValue?: unknown }>, area: string) => {
    if (area !== 'local' || !changes.settings) return;
    const s = changes.settings.newValue as { enabled?: boolean; videoButton?: boolean } | undefined;
    const next = s?.enabled !== false && s?.videoButton !== false;
    enabled = next;
    if (!enabled) hide(true);
    else dismissed = new WeakMap();
  };
  browser.storage?.onChanged?.addListener(onSettings);
  signal.addEventListener('abort', () => {
    try {
      browser.storage?.onChanged?.removeListener(onSettings);
    } catch {
      // Already cut off from the extension.
    }
  });

  button.addEventListener('click', (event) => {
    // The page's player must not take the click as a pause.
    event.stopPropagation();
    event.preventDefault();
    if (menuOpen) closeMenu();
    else void openMenu();
  });
  host.addEventListener('pointerenter', () => clearTimeout(hideTimer));
  host.addEventListener('pointerleave', () => hide());

  document.addEventListener(
    'pointermove',
    (event) => {
      if (event.composedPath().includes(host) || !alive()) return;
      const video = videoAt(event.clientX, event.clientY);
      if (video) show(video);
      else if (target) hide();
    },
    { passive: true, capture: true, signal },
  );
  document.addEventListener(
    'pointerdown',
    (event) => {
      if (menuOpen && !event.composedPath().includes(host)) hide(true);
    },
    { capture: true, signal },
  );
  document.addEventListener(
    'keydown',
    (event) => {
      if (event.key === 'Escape' && menuOpen) {
        closeMenu();
        button.focus();
      }
    },
    { signal },
  );
  addEventListener('scroll', place, { passive: true, capture: true, signal });
  addEventListener('resize', place, { passive: true, signal });
  // A player in full screen shows only what is inside it: the button moves in with it.
  // A bare video in full screen can't hold it, so then there is none.
  document.addEventListener(
    'fullscreenchange',
    () => {
      if (!alive()) return;
      const box = document.fullscreenElement;
      (box && !(box instanceof HTMLVideoElement) ? box : document.documentElement).append(host);
      place();
    },
    { signal },
  );
}
