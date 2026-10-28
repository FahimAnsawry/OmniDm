// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useState } from 'react';
import { browser } from 'wxt/browser';
import type { LinkStatus } from '../../lib/native';
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

async function currentHost(): Promise<string | undefined> {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
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
        <p className="text-neutral-600 dark:text-neutral-400">{STATUS_TEXT[status]}</p>
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
