// SPDX-License-Identifier: GPL-3.0-or-later

// The safe handoff of a browser download to OmniDm (plan §9.3): pause it, offer it to the
// app with its headers and cookies, then remove the browser's copy only if the app took
// it. On a refusal, a timeout or any error the browser simply resumes its own download.

import type { BrowserCookie } from '../../protocol/ts/BrowserCookie';
import type { Capture } from '../../protocol/ts/Capture';
import type { Envelope } from '../../protocol/ts/Envelope';
import type { Message } from '../../protocol/ts/Message';
import type { HeaderCache } from './header-cache';
import { decide } from './rules';
import type { CaptureSettings } from './settings';

/** The parts of `downloads.DownloadItem` the handoff reads. */
export interface DownloadItemLike {
  id: number;
  url: string;
  finalUrl?: string;
  referrer?: string;
  /** Full path chosen by the browser; may still be empty when the download starts. */
  filename?: string;
  mime?: string;
  /** -1 or 0 when unknown. */
  totalBytes?: number;
  fileSize?: number;
  incognito: boolean;
}

export interface CaptureDeps {
  settings: () => Promise<CaptureSettings>;
  downloads: {
    pause(id: number): Promise<void>;
    resume(id: number): Promise<void>;
    cancel(id: number): Promise<void>;
    erase(query: { id: number }): Promise<unknown>;
  };
  /** The browser's cookies for a URL. */
  cookies: (url: string) => Promise<BrowserCookie[]>;
  cache: HeaderCache;
  /** Sends a message to the app and waits for its reply. */
  request: (message: Message, timeoutMs: number) => Promise<Envelope>;
  now: () => number;
}

/** What happened to the download. */
export type CaptureOutcome = 'captured' | 'resumed' | 'left';

/** How long the browser's download stays paused while OmniDm decides. */
const RESULT_TIMEOUT_MS = 10_000;

export async function handleCreated(
  item: DownloadItemLike,
  deps: CaptureDeps,
): Promise<CaptureOutcome> {
  const size = sizeOf(item);
  const decision = decide(
    {
      url: item.url,
      finalUrl: item.finalUrl,
      fileName: baseName(item.filename),
      size,
      incognito: item.incognito,
    },
    await deps.settings(),
  );
  if (!decision.capture) return 'left';

  try {
    await deps.downloads.pause(item.id);
  } catch {
    // Already finished or failed: nothing left to take over.
    return 'left';
  }

  let accepted = false;
  try {
    const capture = await buildCapture(item, size, deps);
    const reply = await deps.request({ type: 'capture', payload: capture }, RESULT_TIMEOUT_MS);
    accepted = reply.type === 'captureResult' && reply.payload.outcome === 'accepted';
  } catch {
    accepted = false;
  }

  if (!accepted) {
    await deps.downloads.resume(item.id).catch(() => undefined);
    return 'resumed';
  }
  // OmniDm has it: the browser's copy goes, and so does its entry in the list.
  await deps.downloads.cancel(item.id).catch(() => undefined);
  await deps.downloads.erase({ id: item.id }).catch(() => undefined);
  return 'captured';
}

async function buildCapture(
  item: DownloadItemLike,
  size: number | undefined,
  deps: CaptureDeps,
): Promise<Capture> {
  const urls = [item.url, item.finalUrl].filter((u): u is string => !!u);
  const seen = deps.cache.lookup(urls, deps.now());
  const capture: Capture = {
    url: item.url,
    headers: seen?.headers ?? [],
    cookies: await cookiesFor(urls, deps),
    private: item.incognito,
    browserDownloadId: item.id,
  };
  if (item.finalUrl && item.finalUrl !== item.url) capture.finalUrl = item.finalUrl;
  if (item.referrer) capture.referrer = item.referrer;
  if (seen?.method && seen.method.toUpperCase() !== 'GET') {
    capture.method = seen.method.toUpperCase();
    if (seen.body) capture.body = seen.body;
  }
  const name = baseName(item.filename);
  if (name) capture.suggestedName = name;
  if (item.mime) capture.mime = item.mime;
  if (size !== undefined) capture.size = size;
  return capture;
}

/** The cookies for every URL of the download, each cookie once. */
async function cookiesFor(urls: string[], deps: CaptureDeps): Promise<BrowserCookie[]> {
  const lists = await Promise.all(urls.map((url) => deps.cookies(url).catch(() => [])));
  const unique = new Map<string, BrowserCookie>();
  for (const cookie of lists.flat()) {
    unique.set(`${cookie.name}|${cookie.domain}|${cookie.path}`, cookie);
  }
  return [...unique.values()];
}

function sizeOf(item: DownloadItemLike): number | undefined {
  for (const size of [item.totalBytes, item.fileSize]) {
    if (size !== undefined && size > 0) return size;
  }
  return undefined;
}

/** `C:\Users\me\Downloads\a.zip` → `a.zip`. */
function baseName(path: string | undefined): string | undefined {
  const name = path?.split(/[\\/]/).pop();
  return name ? name : undefined;
}
