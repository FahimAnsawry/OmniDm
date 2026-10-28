// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import { decide, hostMatches, type DownloadFacts } from './rules';
import { DEFAULT_SETTINGS, type CaptureSettings } from './settings';

const facts = (over: Partial<DownloadFacts> = {}): DownloadFacts => ({
  url: 'https://example.com/files/setup.exe',
  fileName: 'setup.exe',
  size: 5_000_000,
  incognito: false,
  ...over,
});

const settings = (over: Partial<CaptureSettings> = {}): CaptureSettings => ({
  ...DEFAULT_SETTINGS,
  ...over,
});

describe('decide', () => {
  it('captures an ordinary download by default', () => {
    expect(decide(facts(), settings())).toEqual({ capture: true });
  });

  it('leaves everything to the browser when capture is off', () => {
    expect(decide(facts(), settings({ enabled: false }))).toEqual({
      capture: false,
      reason: 'disabled',
    });
  });

  it('never captures links the app cannot fetch', () => {
    for (const url of [
      'blob:https://example.com/0f1e',
      'data:text/plain,hello',
      'file:///C:/a.txt',
      'ftp://example.com/a.zip',
    ]) {
      expect(decide(facts({ url }), settings())).toEqual({ capture: false, reason: 'scheme' });
    }
  });

  it('leaves private-window downloads unless allowed', () => {
    expect(decide(facts({ incognito: true }), settings())).toEqual({
      capture: false,
      reason: 'private',
    });
    expect(decide(facts({ incognito: true }), settings({ capturePrivate: true }))).toEqual({
      capture: true,
    });
  });

  it('respects excluded sites, including their subdomains', () => {
    const rules = settings({ excludedSites: ['example.com'] });
    expect(decide(facts(), rules)).toEqual({ capture: false, reason: 'excludedSite' });
    expect(decide(facts({ url: 'https://dl.example.com/a.zip' }), rules).capture).toBe(false);
    expect(decide(facts({ url: 'https://notexample.com/a.zip' }), rules).capture).toBe(true);
  });

  it('checks the final URL too, after redirects', () => {
    const rules = settings({ excludedSites: ['cdn.example.net'] });
    expect(
      decide(facts({ finalUrl: 'https://cdn.example.net/a.zip' }), rules).capture,
    ).toBe(false);
  });

  it('captures only listed file types when a list is set', () => {
    const rules = settings({ fileTypes: ['zip', 'ISO'] });
    expect(decide(facts({ fileName: 'a.zip' }), rules).capture).toBe(true);
    expect(decide(facts({ fileName: 'ubuntu.iso' }), rules).capture).toBe(true);
    expect(decide(facts({ fileName: 'page.pdf' }), rules)).toEqual({
      capture: false,
      reason: 'fileType',
    });
  });

  it('falls back to the URL for the file type when the name is not known yet', () => {
    const rules = settings({ fileTypes: ['zip'] });
    expect(
      decide(facts({ fileName: undefined, url: 'https://e.com/get/a.zip?x=1' }), rules).capture,
    ).toBe(true);
  });

  it('leaves small files to the browser, but only when the size is known', () => {
    const rules = settings({ minSize: 1_000_000 });
    expect(decide(facts({ size: 999_999 }), rules)).toEqual({
      capture: false,
      reason: 'tooSmall',
    });
    expect(decide(facts({ size: 1_000_000 }), rules).capture).toBe(true);
    expect(decide(facts({ size: undefined }), rules).capture).toBe(true);
  });
});

describe('hostMatches', () => {
  it('matches the site and its subdomains, ignoring case and a leading dot', () => {
    expect(hostMatches('example.com', 'example.com')).toBe(true);
    expect(hostMatches('a.b.example.com', '.Example.COM')).toBe(true);
    expect(hostMatches('badexample.com', 'example.com')).toBe(false);
    expect(hostMatches('example.com', '')).toBe(false);
  });
});
