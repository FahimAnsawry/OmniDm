// SPDX-License-Identifier: GPL-3.0-or-later

// Which browser downloads OmniDm offers to take over (plan §9.3).

import type { CaptureSettings } from './settings';

/** What the browser knows about a new download. */
export interface DownloadFacts {
  url: string;
  /** After redirects, when the browser already knows it. */
  finalUrl?: string;
  fileName?: string;
  /** In bytes; undefined when unknown. */
  size?: number;
  incognito: boolean;
}

export type SkipReason = 'disabled' | 'scheme' | 'private' | 'excludedSite' | 'fileType' | 'tooSmall';

export type Decision = { capture: true } | { capture: false; reason: SkipReason };

/** `host` is `site` or one of its subdomains. */
export function hostMatches(host: string, site: string): boolean {
  const wanted = site.trim().replace(/^\./, '').toLowerCase();
  const actual = host.toLowerCase();
  return wanted !== '' && (actual === wanted || actual.endsWith(`.${wanted}`));
}

function parse(url: string | undefined): URL | undefined {
  if (!url) return undefined;
  try {
    return new URL(url);
  } catch {
    return undefined;
  }
}

/** The extension of a file name or URL path, lowercase, without the dot. */
function extensionOf(name: string): string {
  const base = name.split(/[\\/]/).pop() ?? '';
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}

export function decide(facts: DownloadFacts, settings: CaptureSettings): Decision {
  if (!settings.enabled) return { capture: false, reason: 'disabled' };
  const url = parse(facts.url);
  if (!url || (url.protocol !== 'http:' && url.protocol !== 'https:')) {
    return { capture: false, reason: 'scheme' };
  }
  if (facts.incognito && !settings.capturePrivate) return { capture: false, reason: 'private' };

  const hosts = [url, parse(facts.finalUrl)]
    .filter((u): u is URL => u !== undefined)
    .map((u) => u.hostname);
  if (hosts.some((host) => settings.excludedSites.some((site) => hostMatches(host, site)))) {
    return { capture: false, reason: 'excludedSite' };
  }

  if (settings.fileTypes.length > 0) {
    const name = facts.fileName || parse(facts.finalUrl)?.pathname || url.pathname;
    const wanted = settings.fileTypes.map((type) => type.replace(/^\./, '').toLowerCase());
    if (!wanted.includes(extensionOf(name))) return { capture: false, reason: 'fileType' };
  }

  if (facts.size !== undefined && facts.size < settings.minSize) {
    return { capture: false, reason: 'tooSmall' };
  }
  return { capture: true };
}
