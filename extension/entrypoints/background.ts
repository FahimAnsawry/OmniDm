// SPDX-License-Identifier: GPL-3.0-or-later

// Connects the browser to the OmniDm app: offers new downloads to it (the safe handoff),
// remembers request headers for them, adds the "Download with OmniDm" menu items, and
// notices videos playing on pages.

import { storage } from '#imports';
import { browser, type Browser } from 'wxt/browser';
import type { Capture } from '../../protocol/ts/Capture';
import type { MediaFound } from '../../protocol/ts/MediaFound';
import type { MediaInfo } from '../../protocol/ts/MediaInfo';
import type { MediaQualityPick } from '../../protocol/ts/MediaQualityPick';
import { handleCreated, type CaptureDeps } from '../lib/capture';
import { toBrowserCookie } from '../lib/cookies';
import { encodeRequestBody, HeaderCache, type RequestBodyLike } from '../lib/header-cache';
import { collectLinks, httpLinks } from '../lib/links';
import {
  addFound,
  applyInspection,
  downloadable,
  forFrame,
  inspectPlaylist,
  shown,
  streamKind,
  youtubeFound,
  youtubeVideo,
  type FoundMedia,
  type Inspection,
  type PlaylistKind,
} from '../lib/media';
import { NativeLink, type LinkStatus } from '../lib/native';
import { badgeFor, badgeForTab, shouldOpenSetup, type Badge } from '../lib/notice';
import { loadSettings } from '../lib/settings-store';
import type { FrameVideo } from './video-button';

/** The host's registered name (plan §3). */
const HOST_NAME = 'io.github.fahimansawry.omnidm.host';
const PING_EVERY_MS = 20_000;
const MENU_LINK = 'omnidm-link';
const MENU_ALL = 'omnidm-all-links';
const MENU_SELECTED = 'omnidm-selected-links';
/** A bigger "playlist" is not one; it isn't read. */
const PLAYLIST_LIMIT = 5 * 1024 * 1024;
/** How long what a video offers is remembered for its menu. */
const LOOK_KEPT_MS = 2 * 60_000;

/** Messages from the popup, the setup page and the button on videos. */
export type PopupRequest =
  | { type: 'status' }
  | { type: 'connect' }
  /** The videos found on a tab. */
  | { type: 'media'; tabId: number }
  /** Sends one of them to the app, which asks for its quality. */
  | { type: 'downloadMedia'; tabId: number; url: string }
  /** From the button on a video: what the videos of its frame offer. */
  | { type: 'videoQualities' }
  /** From the button on a video: open the app's window with this quality picked. */
  | { type: 'videoChoose'; url: string; quality: MediaQualityPick };

/** The answer to `downloadMedia`. */
export type MediaReply = { ok: true } | { error: string };

export default defineBackground(() => {
  const cache = new HeaderCache();

  const cookiesFor = async (url: string) =>
    (await browser.cookies.getAll({ url })).map(toBrowserCookie);

  const link = new NativeLink({
    connect: () => browser.runtime.connectNative(HOST_NAME),
    lastError: () => browser.runtime.lastError?.message,
    hello: {
      extensionVersion: browser.runtime.getManifest().version,
      ...browserName(),
      protocols: [1],
    },
    onRequest: async (envelope) =>
      envelope.type === 'cookiesRequest'
        ? { type: 'cookiesResponse', payload: { cookies: await cookiesFor(envelope.payload.url) } }
        : undefined,
  });

  // --- videos playing on pages, by tab ---
  const found = new Map<number, FoundMedia[]>();
  // Kept for the browser session, since the background may be stopped between uses.
  const foundItem = storage.defineItem<Record<string, FoundMedia[]>>('session:media', { fallback: {} });
  const restored = foundItem
    .getValue()
    .then((saved) => {
      for (const [tab, list] of Object.entries(saved)) {
        if (!found.has(Number(tab))) found.set(Number(tab), list);
      }
    })
    .catch(() => undefined);

  // --- the toolbar badge: the "Install OmniDm" notice while the app can't be found, else
  // how many videos the page has ---
  let linkStatus: LinkStatus = link.status;
  const setBadge = (badge: Badge, tabId?: number) => {
    const tab = tabId === undefined ? {} : { tabId };
    void browser.action.setBadgeText({ text: badge.text, ...tab }).catch(() => undefined);
    void browser.action.setBadgeBackgroundColor({ color: badge.color, ...tab }).catch(() => undefined);
    void browser.action.setTitle({ title: badge.title, ...tab }).catch(() => undefined);
  };
  const showTabBadge = (tabId: number) =>
    setBadge(badgeForTab(linkStatus, downloadable(found.get(tabId) ?? [])), tabId);
  const showBadge = (status: LinkStatus) => {
    // Keep the last badge while trying again, so it doesn't flicker.
    if (status === 'connecting') return;
    linkStatus = status;
    setBadge(badgeFor(status));
    for (const tabId of found.keys()) showTabBadge(tabId);
  };
  link.onStatus(showBadge);

  const updateFound = (tabId: number, change: (list: FoundMedia[]) => FoundMedia[]) => {
    const list = change(found.get(tabId) ?? []);
    if (list.length > 0) found.set(tabId, list);
    else found.delete(tabId);
    showTabBadge(tabId);
    void foundItem.setValue(Object.fromEntries(found)).catch(() => undefined);
  };

  /** Reads a playlist once, with the browser's cookies, to tell what it is. */
  const readPlaylist = async (url: string, kind: PlaylistKind): Promise<Inspection> => {
    try {
      const response = await fetch(url, { credentials: 'include', signal: AbortSignal.timeout(10_000) });
      const size = Number(response.headers.get('content-length') ?? 0);
      if (!response.ok || size > PLAYLIST_LIMIT) throw new Error(`HTTP ${response.status}`);
      return inspectPlaylist(kind, await response.text(), response.url || url);
    } catch {
      // Unread: the app reads it again when asked, and says if it can't be downloaded.
      return { state: 'ready', parts: [] };
    }
  };

  /** Puts the Download button into the frame where a video was found. */
  const addButton = async (tabId: number, frameId: number) => {
    if (!(await loadSettings()).videoButton) return;
    await browser.scripting
      .executeScript({ target: { tabId, frameIds: [frameId] }, files: ['/video-button.js'] })
      .catch(() => undefined);
  };

  const noticeVideo = async (tabId: number, frameId: number, url: string, kind: PlaylistKind) => {
    await restored;
    if (found.get(tabId)?.some((item) => item.url === url)) return;
    updateFound(tabId, (list) => addFound(list, url, kind, frameId));
    const inspection = await readPlaylist(url, kind);
    updateFound(tabId, (list) => applyInspection(list, url, inspection));
    if (inspection.state === 'ready') await addButton(tabId, frameId);
  };

  // Only the page's own requests count: the extension's reads have no tab.
  browser.webRequest.onHeadersReceived.addListener(
    (details) => {
      if (details.tabId < 0 || details.statusCode >= 400) return undefined;
      const type = details.responseHeaders?.find((h) => h.name.toLowerCase() === 'content-type')?.value;
      const kind = streamKind(details.url, type);
      if (kind) void noticeVideo(details.tabId, details.frameId, details.url, kind);
      return undefined;
    },
    { urls: ['<all_urls>'], types: ['xmlhttprequest', 'media', 'other'] },
    ['responseHeaders'],
  );
  // YouTube plays no standard playlist: its pages are known by their address instead, and
  // YouTube moves between videos without loading a new page.
  const noticeYoutube = async (tabId: number, frameId: number, video: string) => {
    await restored;
    updateFound(tabId, (list) => [
      // Its page shows one video at a time; an embedded player adds one.
      ...list.filter((item) => item.kind !== 'youtube' || (item.frameId ?? 0) !== frameId),
      youtubeFound(video, frameId),
    ]);
    await addButton(tabId, frameId);
  };
  browser.tabs.onUpdated.addListener((tabId, change, tab) => {
    if (change.url) {
      const video = youtubeVideo(change.url);
      if (video) {
        void noticeYoutube(tabId, 0, video);
      } else if (change.status === 'loading' && found.has(tabId)) {
        // Another page: what the last one played is gone.
        updateFound(tabId, () => []);
      } else if (found.get(tabId)?.some((item) => item.kind === 'youtube' && !item.frameId)) {
        // Left the video for another YouTube page.
        updateFound(tabId, (list) => list.filter((item) => item.kind !== 'youtube' || item.frameId));
      }
    }
    // The page is ready for the button now.
    const video = change.status === 'complete' && tab.url ? youtubeVideo(tab.url) : undefined;
    if (video) void noticeYoutube(tabId, 0, video);
  });
  // An embedded YouTube player, once its frame has loaded.
  browser.webRequest.onCompleted.addListener(
    (details) => {
      const video = details.tabId >= 0 ? youtubeVideo(details.url) : undefined;
      if (video) void noticeYoutube(details.tabId, details.frameId, video);
    },
    {
      urls: ['*://*.youtube.com/embed/*', '*://*.youtube-nocookie.com/embed/*'],
      types: ['sub_frame'],
    },
  );
  browser.tabs.onRemoved.addListener((tabId) => {
    if (found.has(tabId)) updateFound(tabId, () => []);
  });

  // --- request headers and POST bodies, for the downloads they start ---
  const remember = (details: Browser.webRequest.OnSendHeadersDetails) => {
    const headers = (details.requestHeaders ?? [])
      .filter((h) => h.value !== undefined)
      .map((h) => ({ name: h.name, value: h.value! }));
    cache.rememberHeaders(details.url, headers, Date.now());
  };
  try {
    // Chromium hides Referer and Cookie without `extraHeaders`.
    browser.webRequest.onSendHeaders.addListener(remember, { urls: ['<all_urls>'] }, [
      'requestHeaders',
      'extraHeaders',
    ]);
  } catch {
    // Firefox has no `extraHeaders` and shows every header anyway.
    browser.webRequest.onSendHeaders.addListener(remember, { urls: ['<all_urls>'] }, [
      'requestHeaders',
    ]);
  }
  browser.webRequest.onBeforeRequest.addListener(
    (details) => {
      if (details.method !== 'GET') {
        const body = encodeRequestBody(details.requestBody as RequestBodyLike | undefined);
        cache.rememberBody(details.url, details.method, body, Date.now());
      }
      return undefined;
    },
    { urls: ['<all_urls>'] },
    ['requestBody'],
  );

  // --- the handoff ---
  const deps: CaptureDeps = {
    settings: loadSettings,
    downloads: {
      pause: (id) => browser.downloads.pause(id),
      resume: (id) => browser.downloads.resume(id),
      cancel: (id) => browser.downloads.cancel(id),
      erase: (query) => browser.downloads.erase(query),
    },
    cookies: cookiesFor,
    cache,
    request: (message, timeoutMs) => link.request(message, timeoutMs),
    now: () => Date.now(),
  };
  browser.downloads.onCreated.addListener((item) => {
    void handleCreated(item, deps);
  });

  // --- context menus ---
  browser.runtime.onInstalled.addListener((details) => {
    // Just installed and the app isn't on this PC: explain what else is needed, once.
    if (details.reason === 'install') {
      void link.open().then(() => {
        if (shouldOpenSetup(details.reason, link.status)) {
          void browser.tabs.create({ url: browser.runtime.getURL('/setup.html') });
        }
      });
    }
    browser.contextMenus.create({ id: MENU_LINK, title: 'Download with OmniDm', contexts: ['link'] });
    browser.contextMenus.create({
      id: MENU_ALL,
      title: 'Download all links with OmniDm',
      contexts: ['page'],
    });
    browser.contextMenus.create({
      id: MENU_SELECTED,
      title: 'Download selected links with OmniDm',
      contexts: ['selection'],
    });
  });
  browser.contextMenus.onClicked.addListener(async (info, tab) => {
    let urls: string[] = [];
    if (info.menuItemId === MENU_LINK && info.linkUrl) {
      urls = httpLinks([info.linkUrl]);
    } else if ((info.menuItemId === MENU_ALL || info.menuItemId === MENU_SELECTED) && tab?.id) {
      // Runs once, only because the user picked the menu item (plan §9.2).
      const [result] = await browser.scripting.executeScript({
        target: { tabId: tab.id },
        func: collectLinks,
        args: [info.menuItemId === MENU_SELECTED],
      });
      urls = httpLinks((result?.result as string[] | undefined) ?? []);
    }
    if (urls.length === 0) return;
    const items: Capture[] = await Promise.all(
      urls.map(async (url) => ({
        url,
        referrer: info.pageUrl,
        pageUrl: info.pageUrl,
        headers: [],
        cookies: await cookiesFor(url).catch(() => []),
        private: tab?.incognito ?? false,
        browserDownloadId: 0,
      })),
    );
    await link.request({ type: 'captureMany', payload: { items } }, 10_000).catch(() => undefined);
  });

  /** A found video as the app wants it: with its page, headers and cookies. */
  const mediaFound = async (tabId: number, item: FoundMedia): Promise<MediaFound> => {
    const tab = await browser.tabs.get(tabId).catch(() => undefined);
    return {
      kind: item.kind,
      url: item.url,
      pageUrl: tab?.url,
      title: tab?.title,
      headers: cache.lookup([item.url], Date.now())?.headers ?? [],
      cookies: await cookiesFor(item.url).catch(() => []),
      private: tab?.incognito ?? false,
    };
  };

  /** What a video offers, asked of the app (which reads its playlist), kept a while. */
  const looks = new Map<string, { at: number; video: FrameVideo }>();
  const lookAt = async (tabId: number, item: FoundMedia): Promise<FrameVideo> => {
    const kept = looks.get(item.url);
    if (kept && Date.now() - kept.at < LOOK_KEPT_MS) return kept.video;
    let video: FrameVideo;
    try {
      const payload = await mediaFound(tabId, item);
      // yt-dlp can take a while, and the first time it is downloaded too.
      const timeout = item.kind === 'youtube' ? 150_000 : 30_000;
      const reply = await link.request({ type: 'mediaLook', payload }, timeout);
      if (reply.type === 'mediaInfo') video = { url: item.url, info: reply.payload as MediaInfo };
      else if (reply.type === 'error') video = { url: item.url, error: reply.payload.message };
      else video = { url: item.url, error: 'OmniDm gave an unexpected answer.' };
    } catch (e) {
      // Not kept: OmniDm may be back next time.
      return { url: item.url, error: e instanceof Error ? e.message : String(e) };
    }
    looks.set(item.url, { at: Date.now(), video });
    return video;
  };

  /** Sends a found video to the app, which opens its quality window (with `quality`
   * picked, when the button on the video chose one). */
  const sendVideo = async (tabId: number, url: string, quality?: MediaQualityPick): Promise<MediaReply> => {
    await restored;
    const item = found.get(tabId)?.find((media) => media.url === url);
    if (!item) return { error: 'That video is no longer on the page. Play it again.' };
    const payload: MediaFound = { ...(await mediaFound(tabId, item)), quality };
    try {
      const reply = await link.request({ type: 'media', payload }, 10_000);
      return reply.type === 'error' ? { error: reply.payload.message } : { ok: true };
    } catch (e) {
      return { error: e instanceof Error ? e.message : String(e) };
    }
  };

  // --- the popup and the button on videos ---
  browser.runtime.onMessage.addListener((message: PopupRequest, sender) => {
    if (message?.type === 'status') return Promise.resolve({ status: link.status });
    if (message?.type === 'connect') {
      return link.open().then(() => ({ status: link.status }));
    }
    if (message?.type === 'media') {
      return restored.then(() => ({ videos: shown(found.get(message.tabId) ?? []) }));
    }
    if (message?.type === 'downloadMedia') return sendVideo(message.tabId, message.url);
    const tabId = sender.tab?.id;
    if (message?.type === 'videoQualities' && tabId !== undefined) {
      return restored.then(async () => {
        const here = forFrame(found.get(tabId) ?? [], sender.frameId ?? 0);
        return { videos: await Promise.all(here.map((item) => lookAt(tabId, item))) };
      });
    }
    if (message?.type === 'videoChoose' && tabId !== undefined) {
      return sendVideo(tabId, message.url, message.quality);
    }
    return undefined;
  });

  // Keep the connection (and the service worker) alive while OmniDm runs.
  setInterval(() => {
    if (link.status === 'connected') {
      link.request({ type: 'ping' }, 5_000).catch(() => undefined);
    }
  }, PING_EVERY_MS);
  void link.open();
});

/** The browser's name and major version, from its user agent. */
function browserName(): { browser: string; browserVersion: string } {
  const agent = navigator.userAgent;
  for (const [name, pattern] of [
    ['edge', /Edg\/(\d+)/],
    ['firefox', /Firefox\/(\d+)/],
    ['chrome', /Chrome\/(\d+)/],
  ] as const) {
    const match = agent.match(pattern);
    if (match) return { browser: name, browserVersion: match[1] ?? '' };
  }
  return { browser: 'unknown', browserVersion: '' };
}
