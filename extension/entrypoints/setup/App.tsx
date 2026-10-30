// SPDX-License-Identifier: GPL-3.0-or-later

// Opened once, right after the extension is installed on a PC without the OmniDM app:
// says what else is needed, and notices by itself when the app is there.

import { type ReactNode, useEffect, useState } from 'react';
import { browser } from 'wxt/browser';
import type { LinkStatus } from '../../lib/native';
import { DOWNLOAD_PAGE } from '../../lib/notice';

/** How often the page looks for the app. */
const CHECK_EVERY_MS = 3_000;

function App() {
  const [status, setStatus] = useState<LinkStatus>('connecting');

  useEffect(() => {
    let stopped = false;
    const check = async () => {
      try {
        const reply: { status: LinkStatus } = await browser.runtime.sendMessage({ type: 'connect' });
        if (!stopped) setStatus(reply.status);
        return reply.status;
      } catch {
        return 'disconnected' as const;
      }
    };
    let timer: ReturnType<typeof setTimeout>;
    const loop = async () => {
      const found = (await check()) === 'connected';
      if (!found && !stopped) timer = setTimeout(loop, CHECK_EVERY_MS);
    };
    void loop();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);

  const found = status === 'connected';

  return (
    <main className="min-h-screen bg-neutral-50 px-6 py-12 font-sans text-[15px] text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <div className="mx-auto max-w-xl space-y-6">
        <header className="space-y-2">
          <p className="text-sm font-medium text-sky-700 dark:text-sky-400">OmniDM extension installed</p>
          <h1 className="text-2xl font-semibold">
            {found ? "You're all set" : 'One more step: install the OmniDM app'}
          </h1>
          <p className="text-neutral-600 dark:text-neutral-400">
            This extension hands your downloads to the OmniDM app on this PC, which downloads them faster and
            can pause and resume them. {found ? '' : "The app isn't on this PC yet, so for now downloads stay in the browser."}
          </p>
        </header>

        <div
          role="status"
          aria-live="polite"
          className={`flex items-center gap-3 rounded-lg border px-4 py-3 text-sm ${
            found
              ? 'border-emerald-300 bg-emerald-50 text-emerald-900 dark:border-emerald-800 dark:bg-emerald-950 dark:text-emerald-100'
              : 'border-neutral-200 bg-white text-neutral-700 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300'
          }`}
        >
          <span
            className={`size-2.5 shrink-0 rounded-full ${
              found ? 'bg-emerald-500' : status === 'notRunning' ? 'bg-amber-500' : 'bg-neutral-400'
            }`}
          />
          {found
            ? 'OmniDM is connected. Downloads you start in the browser now go to OmniDM. You can close this tab.'
            : status === 'notRunning'
              ? "OmniDM is installed but didn't start. Start OmniDM from the Start menu; this page will notice."
              : 'Looking for OmniDM… This page notices by itself once it is installed.'}
        </div>

        {!found && (
          <ol className="space-y-4">
            <Step n={1} title="Download OmniDM for Windows">
              <a
                href={DOWNLOAD_PAGE}
                target="_blank"
                rel="noreferrer"
                className="mt-2 inline-block rounded-md bg-sky-600 px-4 py-2 text-sm font-medium text-white hover:bg-sky-700"
              >
                Get OmniDM
              </a>
            </Step>
            <Step n={2} title="Run the installer">
              It needs no administrator rights, and it connects OmniDM to Chrome, Edge and Firefox.
            </Step>
            <Step n={3} title="Come back to this tab">It turns green once OmniDM is found.</Step>
          </ol>
        )}

        {!found && (
          <details className="text-sm text-neutral-600 dark:text-neutral-400">
            <summary className="cursor-pointer">Running OmniDM from its source code?</summary>
            <p className="mt-2">
              Connect it to your browsers once: in PowerShell, open the folder that has the OmniDM app and
              run{' '}
              <code className="rounded bg-neutral-200 px-1 py-0.5 text-xs dark:bg-neutral-800">
                .\omnidm-host.exe --register
              </code>
              .
            </p>
          </details>
        )}
      </div>
    </main>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-4">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-sky-100 text-sm font-semibold text-sky-800 dark:bg-sky-900 dark:text-sky-200">
        {n}
      </span>
      <div className="pt-0.5">
        <p className="font-medium">{title}</p>
        <div className="text-sm text-neutral-600 dark:text-neutral-400">{children}</div>
      </div>
    </li>
  );
}

export default App;
