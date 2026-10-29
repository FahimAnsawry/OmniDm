// SPDX-License-Identifier: GPL-3.0-or-later

// Connects the browser to the OmniDm app: offers new downloads to it (the safe handoff),
// remembers request headers for them, and adds the "Download with OmniDm" menu items.

import { browser, type Browser } from 'wxt/browser';
import type { Capture } from '../../protocol/ts/Capture';
import { handleCreated, type CaptureDeps } from '../lib/capture';
import { toBrowserCookie } from '../lib/cookies';
import { encodeRequestBody, HeaderCache, type RequestBodyLike } from '../lib/header-cache';
import { collectLinks, httpLinks } from '../lib/links';
import { NativeLink, type LinkStatus } from '../lib/native';
import { badgeFor, shouldOpenSetup } from '../lib/notice';
import { loadSettings } from '../lib/settings-store';

/** The host's registered name (plan §3). */
const HOST_NAME = 'io.github.fahimansawry.omnidm.host';
const PING_EVERY_MS = 20_000;
const MENU_LINK = 'omnidm-link';
const MENU_ALL = 'omnidm-all-links';
const MENU_SELECTED = 'omnidm-selected-links';

/** Messages from the popup and the setup page. */
export type PopupRequest = { type: 'status' } | { type: 'connect' };

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

  // --- the "Install OmniDm" notice: a badge while the app can't be found ---
  const showBadge = (status: LinkStatus) => {
    // Keep the last badge while trying again, so it doesn't flicker.
    if (status === 'connecting') return;
    const badge = badgeFor(status);
    void browser.action.setBadgeText({ text: badge.text });
    void browser.action.setBadgeBackgroundColor({ color: badge.color });
    void browser.action.setTitle({ title: badge.title });
  };
  link.onStatus(showBadge);

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

  // --- the popup ---
  browser.runtime.onMessage.addListener((message: PopupRequest) => {
    if (message?.type === 'status') return Promise.resolve({ status: link.status });
    if (message?.type === 'connect') {
      return link.open().then(() => ({ status: link.status }));
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
