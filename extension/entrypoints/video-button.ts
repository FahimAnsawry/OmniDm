// SPDX-License-Identifier: GPL-3.0-or-later

// The Download button on a video. The background puts this script into a frame only once
// it found a video playlist there. The button shows over the top-right corner of a video
// while the pointer is on it; it opens a menu of the video's qualities, and picking one
// opens OmniDm's video window with that quality chosen.
//
// Everything is drawn in a closed shadow root, so the page's styles can't reach it, and
// text from the network is only ever set as text.

import { browser } from 'wxt/browser';
import type { MediaInfo } from '../../protocol/ts/MediaInfo';
import { mediaName, qualityLabel, sizeLabel } from '../lib/media';

/** Smaller videos (previews, ads, avatars) get no button. */
const MIN_WIDTH = 240;
const MIN_HEIGHT = 135;
/** How long the button stays after the pointer leaves the video. */
const HIDE_AFTER_MS = 1500;
/** How long "Opened in OmniDm" shows before the menu closes. */
const DONE_SHOWN_MS = 1500;

/** A video of this frame and what it offers, as the background answers `videoQualities`. */
export type FrameVideo = { url: string; info?: MediaInfo; error?: string };

const CSS = `
:host { all: initial; }
.wrap { position: relative; font: 13px/1.35 system-ui, -apple-system, "Segoe UI", sans-serif; }
.button {
  display: flex; align-items: center; justify-content: center; width: 34px; height: 34px;
  padding: 0; border: 0; border-radius: 999px; background: rgb(2 132 199 / 0.92); color: #fff;
  cursor: pointer; box-shadow: 0 2px 8px rgb(0 0 0 / 0.3);
}
.button:hover { background: rgb(3 105 161); }
.button:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
.button svg { width: 18px; height: 18px; }
.menu {
  position: absolute; top: calc(100% + 6px); right: 0; min-width: 240px; max-width: 340px;
  max-height: 320px; overflow-y: auto; padding: 4px; border-radius: 10px; background: #fff;
  color: #171717; box-shadow: 0 8px 28px rgb(0 0 0 / 0.28);
}
.menu[hidden] { display: none; }
.title { padding: 6px 10px 4px; font-size: 11px; font-weight: 600; color: #737373; text-transform: uppercase; letter-spacing: 0.04em; }
.item {
  display: flex; width: 100%; justify-content: space-between; gap: 16px; padding: 7px 10px;
  border: 0; border-radius: 6px; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer;
}
.item:hover, .item:focus-visible { background: #e0f2fe; outline: none; }
.item .size { color: #737373; font-variant-numeric: tabular-nums; }
.note { padding: 8px 10px; color: #525252; }
.error { padding: 8px 10px; color: #b91c1c; }
@media (prefers-color-scheme: dark) {
  .menu { background: #262626; color: #f5f5f5; }
  .title, .item .size { color: #a3a3a3; }
  .item:hover, .item:focus-visible { background: rgb(12 74 110 / 0.6); }
  .note { color: #d4d4d4; }
  .error { color: #f87171; }
}
`;

const ICON =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 15V3"/><path d="m7 10 5 5 5-5"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/></svg>';

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
  root.innerHTML = `<style>${CSS}</style><div class="wrap"><button class="button" type="button" aria-haspopup="menu" aria-expanded="false" aria-label="Download with OmniDm" title="Download with OmniDm">${ICON}</button><div class="menu" role="menu" hidden></div></div>`;
  const button = root.querySelector<HTMLButtonElement>('.button')!;
  const menu = root.querySelector<HTMLDivElement>('.menu')!;
  document.documentElement.append(host);

  let target: HTMLVideoElement | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let menuOpen = false;

  const bigEnough = (rect: DOMRect) => rect.width >= MIN_WIDTH && rect.height >= MIN_HEIGHT;

  /** The video under a point, if it is big enough for a button. */
  const videoAt = (x: number, y: number) => {
    for (const video of document.querySelectorAll('video')) {
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
    host.style.top = `${Math.max(rect.top, 0) + 10}px`;
    host.style.left = `${Math.min(rect.right, innerWidth) - 10}px`;
    host.style.transform = 'translateX(-100%)';
  };

  const show = (video: HTMLVideoElement) => {
    clearTimeout(hideTimer);
    if (target !== video) closeMenu();
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

  /** Fills the menu: a heading, then lines of text or quality items. */
  const fill = (...parts: HTMLElement[]) => {
    const title = document.createElement('div');
    title.className = 'title';
    title.textContent = 'Download with OmniDm';
    menu.replaceChildren(title, ...parts);
  };

  const line = (className: string, text: string) => {
    const element = document.createElement('div');
    element.className = className;
    element.textContent = text;
    return element;
  };

  const choose = async (url: string, quality: { height?: number; bandwidth: number }) => {
    fill(line('note', 'Opening OmniDm…'));
    try {
      const reply: { ok: true } | { error: string } = await browser.runtime.sendMessage({
        type: 'videoChoose',
        url,
        quality,
      });
      if ('error' in reply) {
        fill(line('error', reply.error));
        return;
      }
      fill(line('note', 'Opened in OmniDm'));
      setTimeout(() => hide(true), DONE_SHOWN_MS);
    } catch {
      fill(line('error', "OmniDm couldn't be reached."));
    }
  };

  const openMenu = async () => {
    menuOpen = true;
    menu.hidden = false;
    button.setAttribute('aria-expanded', 'true');
    fill(line('note', 'Reading the qualities…'));
    let videos: FrameVideo[];
    try {
      const reply: { videos: FrameVideo[] } = await browser.runtime.sendMessage({ type: 'videoQualities' });
      videos = reply.videos;
    } catch {
      fill(line('error', "OmniDm couldn't be reached."));
      return;
    }
    if (!menuOpen) return;
    if (videos.length === 0) {
      fill(line('note', 'No video to download here any more.'));
      return;
    }
    const parts: HTMLElement[] = [];
    for (const video of videos) {
      // Several videos on one page: each under its own name.
      if (videos.length > 1) parts.push(line('title', mediaName(video.url)));
      if (!video.info) {
        parts.push(line('error', video.error ?? "This video can't be downloaded."));
        continue;
      }
      for (const quality of video.info.qualities) {
        const item = document.createElement('button');
        item.type = 'button';
        item.className = 'item';
        item.setAttribute('role', 'menuitem');
        item.append(
          line('name', qualityLabel(quality)),
          line('size', quality.size != null ? sizeLabel(quality.size) : ''),
        );
        item.addEventListener('click', (event) => {
          event.stopPropagation();
          void choose(video.url, { height: quality.height, bandwidth: quality.bandwidth });
        });
        parts.push(item);
      }
    }
    fill(...parts);
    menu.querySelector<HTMLButtonElement>('.item')?.focus();
  };

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
      if (event.composedPath().includes(host)) return;
      const video = videoAt(event.clientX, event.clientY);
      if (video) show(video);
      else if (target) hide();
    },
    { passive: true, capture: true },
  );
  document.addEventListener(
    'pointerdown',
    (event) => {
      if (menuOpen && !event.composedPath().includes(host)) hide(true);
    },
    { capture: true },
  );
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && menuOpen) {
      closeMenu();
      button.focus();
    }
  });
  addEventListener('scroll', place, { passive: true, capture: true });
  addEventListener('resize', place, { passive: true });
  // A player in full screen shows only what is inside it: the button moves in with it.
  // A bare video in full screen can't hold it, so then there is none.
  document.addEventListener('fullscreenchange', () => {
    const box = document.fullscreenElement;
    (box && !(box instanceof HTMLVideoElement) ? box : document.documentElement).append(host);
    place();
  });
}
