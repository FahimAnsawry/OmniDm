// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useState } from 'react';
import { browser } from 'wxt/browser';
import type { MediaReply } from '../background';
import { mediaName, type FoundMedia, type MediaState } from '../../lib/media';
import type { LinkStatus } from '../../lib/native';
import { DOWNLOAD_PAGE } from '../../lib/notice';
import { hostMatches } from '../../lib/rules';
import { DEFAULT_SETTINGS, type CaptureSettings } from '../../lib/settings';
import { loadSettings, settingsItem } from '../../lib/settings-store';

const STATUS_TEXT: Record<LinkStatus, string> = {
  connected: 'Connected to OmniDm',
  connecting: 'Connecting to OmniDm…',
  disconnected: 'Not connected to OmniDm',
  notRunning: "OmniDm isn't running. Downloads stay in the browser.",
  notInstalled: "OmniDm isn't installed. Downloads stay in the browser.",
};

const STATUS_DOT: Record<LinkStatus, string> = {
  connected: 'bg-emerald-500',
  connecting: 'bg-amber-400',
  disconnected: 'bg-neutral-400',
  notRunning: 'bg-amber-500',
  notInstalled: 'bg-red-500',
};

const KIND_NOTE: Record<FoundMedia['kind'], string> = {
  hls: 'HLS video',
  dash: 'DASH video',
  youtube: 'Read with yt-dlp',
};

/** Why a found video can't be downloaded, or that it is still being read. */
const MEDIA_NOTE: Partial<Record<MediaState, string>> = {
  checking: 'Checking…',
  protected: "Protected (DRM), can't be downloaded",
  live: "Live stream, can't be downloaded yet",
};

async function currentTab() {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  return tab;
}

async function currentHost(): Promise<string | undefined> {
  const tab = await currentTab();
  try {
    const url = new URL(tab?.url ?? '');
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.hostname : undefined;
  } catch {
    return undefined;
  }
}

function App() {
  const [status, setStatus] = useState<LinkStatus>('connecting');
  const [settings, setSettings] = useState<CaptureSettings>(DEFAULT_SETTINGS);
  const [host, setHost] = useState<string>();
  const [tabId, setTabId] = useState<number>();
  const [videos, setVideos] = useState<FoundMedia[]>([]);
  const [sending, setSending] = useState<string>();
  const [mediaError, setMediaError] = useState<string>();

  useEffect(() => {
    currentTab().then(async (tab) => {
      if (tab?.id === undefined) return;
      setTabId(tab.id);
      const reply: { videos: FoundMedia[] } = await browser.runtime.sendMessage({
        type: 'media',
        tabId: tab.id,
      });
      setVideos(reply.videos);
    });
  }, []);

  useEffect(() => {
    browser.runtime
      .sendMessage({ type: 'status' })
      .then((reply: { status: LinkStatus }) => setStatus(reply.status))
      .catch(() => setStatus('disconnected'));
    loadSettings().then(setSettings);
    currentHost().then(setHost);
  }, []);

  const save = async (next: CaptureSettings) => {
    setSettings(next);
    await settingsItem.setValue(next);
  };

  const connect = async () => {
    setStatus('connecting');
    const reply: { status: LinkStatus } = await browser.runtime.sendMessage({ type: 'connect' });
    setStatus(reply.status);
  };

  const download = async (url: string) => {
    if (tabId === undefined) return;
    setSending(url);
    setMediaError(undefined);
    const reply: MediaReply = await browser.runtime.sendMessage({ type: 'downloadMedia', tabId, url });
    setSending(undefined);
    // The app's window takes it from here.
    if ('ok' in reply) window.close();
    else setMediaError(reply.error);
  };

  const excluded = host !== undefined && settings.excludedSites.some((site) => hostMatches(host, site));
  const toggleSite = () => {
    if (!host) return;
    const excludedSites = excluded
      ? settings.excludedSites.filter((site) => !hostMatches(host, site))
      : [...settings.excludedSites, host];
    void save({ ...settings, excludedSites });
  };

  return (
    <main className="w-80 bg-white font-sans text-sm text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      <header className="flex items-center gap-2 border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
        <span className={`size-2.5 rounded-full ${STATUS_DOT[status]}`} />
        <h1 className="text-base font-semibold">OmniDm</h1>
      </header>

      <section className="space-y-3 px-4 py-3">
        {status === 'notInstalled' ? (
          <div
            role="alert"
            className="space-y-2 rounded border border-red-200 bg-red-50 p-3 text-red-950 dark:border-red-900 dark:bg-red-950 dark:text-red-100"
          >
            <p className="font-medium">Install the OmniDm app</p>
            <p className="text-red-900/80 dark:text-red-100/80">
              This extension needs the OmniDm app on this PC. Until it is installed, downloads stay in the
              browser.
            </p>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => void browser.tabs.create({ url: DOWNLOAD_PAGE })}
                className="rounded bg-red-600 px-3 py-1 font-medium text-white hover:bg-red-700"
              >
                Get OmniDm
              </button>
              <button
                type="button"
                onClick={() => void browser.tabs.create({ url: browser.runtime.getURL('/setup.html') })}
                className="rounded border border-red-300 px-3 py-1 hover:bg-red-100 dark:border-red-800 dark:hover:bg-red-900"
              >
                How to set up
              </button>
              <button
                type="button"
                onClick={connect}
                className="rounded border border-red-300 px-3 py-1 hover:bg-red-100 dark:border-red-800 dark:hover:bg-red-900"
              >
                Check again
              </button>
            </div>
          </div>
        ) : (
          <p className="text-neutral-600 dark:text-neutral-400">{STATUS_TEXT[status]}</p>
        )}
        {(status === 'notRunning' || status === 'disconnected') && (
          <button
            type="button"
            onClick={connect}
            className="rounded border border-neutral-300 px-3 py-1 hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
          >
            Connect
          </button>
        )}

        <label className="flex items-center justify-between gap-3">
          <span>Send downloads to OmniDm</span>
          <input
            type="checkbox"
            className="size-4 accent-sky-600"
            checked={settings.enabled}
            onChange={(e) => void save({ ...settings, enabled: e.target.checked })}
          />
        </label>

        {videos.length > 0 && (
          <div className="space-y-1.5">
            <h2 className="text-xs font-medium text-neutral-500 dark:text-neutral-400">Videos on this page</h2>
            <ul className="divide-y divide-neutral-200 rounded border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
              {videos.map((video) => (
                <li key={video.url} className="flex items-center gap-2 px-2.5 py-1.5">
                  <div className="min-w-0 flex-1">
                    <p className="truncate" title={video.url}>
                      {mediaName(video.url)}
                    </p>
                    <p className="text-xs text-neutral-500 dark:text-neutral-400">
                      {MEDIA_NOTE[video.state] ?? KIND_NOTE[video.kind]}
                    </p>
                  </div>
                  {video.state === 'ready' && (
                    <button
                      type="button"
                      onClick={() => void download(video.url)}
                      disabled={sending !== undefined}
                      className="shrink-0 rounded bg-sky-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
                    >
                      {sending === video.url ? 'Opening…' : 'Download'}
                    </button>
                  )}
                </li>
              ))}
            </ul>
            {mediaError && (
              <p role="alert" className="text-xs text-red-700 dark:text-red-400">
                {mediaError}
              </p>
            )}
          </div>
        )}

        {host && (
          <button
            type="button"
            onClick={toggleSite}
            className="w-full rounded border border-neutral-300 px-3 py-1.5 text-left hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
          >
            {excluded ? `Send downloads from ${host} again` : `Never send downloads from ${host}`}
          </button>
        )}
      </section>

      <footer className="border-t border-neutral-200 px-4 py-2 dark:border-neutral-800">
        <button
          type="button"
          onClick={() => browser.runtime.openOptionsPage()}
          className="text-sky-700 hover:underline dark:text-sky-400"
        >
          Settings
        </button>
      </footer>
    </main>
  );
}

export default App;
