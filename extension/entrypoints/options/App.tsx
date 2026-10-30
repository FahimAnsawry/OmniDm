// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useState } from 'react';
import { DEFAULT_SETTINGS, type CaptureSettings } from '../../lib/settings';
import { loadSettings, settingsItem } from '../../lib/settings-store';

const MB = 1024 * 1024;

/** Splits user input into trimmed, non-empty entries. */
function entries(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((entry) => entry.trim())
    .filter((entry) => entry !== '');
}

function App() {
  const [settings, setSettings] = useState<CaptureSettings>(DEFAULT_SETTINGS);
  const [minSize, setMinSize] = useState('0');
  const [fileTypes, setFileTypes] = useState('');
  const [sites, setSites] = useState('');
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    loadSettings().then((loaded) => {
      setSettings(loaded);
      setMinSize(String(loaded.minSize / MB));
      setFileTypes(loaded.fileTypes.join(', '));
      setSites(loaded.excludedSites.join('\n'));
    });
  }, []);

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    const megabytes = Number.parseFloat(minSize);
    const next: CaptureSettings = {
      ...settings,
      minSize: Number.isFinite(megabytes) && megabytes > 0 ? Math.round(megabytes * MB) : 0,
      fileTypes: entries(fileTypes).map((type) => type.replace(/^\./, '').toLowerCase()),
      excludedSites: entries(sites).map((site) => site.toLowerCase()),
    };
    setSettings(next);
    await settingsItem.setValue(next);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const field = 'mt-1 w-full rounded border border-neutral-300 bg-white px-2 py-1.5 dark:border-neutral-700 dark:bg-neutral-800';

  return (
    <main className="min-h-screen bg-neutral-100 font-sans text-sm text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      <form onSubmit={save} className="mx-auto max-w-xl space-y-6 px-6 py-8">
        <h1 className="text-xl font-semibold">OmniDM settings</h1>

        <label className="flex items-center justify-between gap-3">
          <span>Send downloads to OmniDM</span>
          <input
            type="checkbox"
            className="size-4 accent-sky-600"
            checked={settings.enabled}
            onChange={(e) => setSettings({ ...settings, enabled: e.target.checked })}
          />
        </label>

        <label className="block">
          <span className="font-medium">Minimum size (MB)</span>
          <input
            type="number"
            min="0"
            step="any"
            className={field}
            value={minSize}
            onChange={(e) => setMinSize(e.target.value)}
          />
          <span className="mt-1 block text-neutral-500">
            Smaller downloads stay in the browser. 0 sends every download.
          </span>
        </label>

        <label className="block">
          <span className="font-medium">File types</span>
          <input
            className={field}
            placeholder="zip, iso, mp4"
            value={fileTypes}
            onChange={(e) => setFileTypes(e.target.value)}
          />
          <span className="mt-1 block text-neutral-500">
            Only these types go to OmniDM. Leave empty for every type.
          </span>
        </label>

        <label className="block">
          <span className="font-medium">Sites whose downloads stay in the browser</span>
          <textarea
            rows={5}
            className={field}
            placeholder={'example.com\nintranet.local'}
            value={sites}
            onChange={(e) => setSites(e.target.value)}
          />
          <span className="mt-1 block text-neutral-500">One per line. Subdomains are included.</span>
        </label>

        <label className="flex items-center justify-between gap-3">
          <span>Also send downloads from private windows</span>
          <input
            type="checkbox"
            className="size-4 accent-sky-600"
            checked={settings.capturePrivate}
            onChange={(e) => setSettings({ ...settings, capturePrivate: e.target.checked })}
          />
        </label>

        <label className="flex items-center justify-between gap-3">
          <span>Show a Download button on videos OmniDM finds</span>
          <input
            type="checkbox"
            className="size-4 accent-sky-600"
            checked={settings.videoButton}
            onChange={(e) => setSettings({ ...settings, videoButton: e.target.checked })}
          />
        </label>

        <div className="flex items-center gap-3">
          <button
            type="submit"
            className="rounded bg-sky-700 px-4 py-1.5 font-medium text-white hover:bg-sky-800"
          >
            Save
          </button>
          {saved && <span className="text-emerald-700 dark:text-emerald-400">Saved</span>}
        </div>
      </form>
    </main>
  );
}

export default App;
