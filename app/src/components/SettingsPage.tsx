// SPDX-License-Identifier: GPL-3.0-or-later

// Settings: each change is saved and applied at once.

import { type ReactNode, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import { type AppSettings, api, errorText } from "../lib/api";
import { CategoriesEditor } from "./CategoriesEditor";
import { FolderField, selectClass } from "./Fields";
import { Icon } from "./Icon";

type Section = "general" | "connections" | "categories";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "general", label: "General" },
  { id: "connections", label: "Connections" },
  { id: "categories", label: "Categories" },
];

const CONNECTIONS = [1, 2, 4, 6, 8, 12, 16, 24, 32];

export function SettingsPage({
  categories,
  onChanged,
  onClose,
}: {
  categories: CategorySummary[];
  /** Categories changed: the sidebar reloads. */
  onChanged: () => void;
  onClose: () => void;
}) {
  const [section, setSection] = useState<Section>("general");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [folder, setFolder] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    () =>
      api
        .getSettings()
        .then((s) => {
          setSettings(s);
          setFolder(s.downloadsFolder);
        })
        .catch((e) => setError(errorText(e))),
    [],
  );

  useEffect(() => {
    load();
    // Changed from the tray, or by the capture prompt's "Don't ask again".
    const stop = listen("settings-changed", () => load());
    return () => void stop.then((f) => f());
  }, [load]);

  const save = (change: Partial<AppSettings>) => {
    if (!settings) return;
    const next = { ...settings, ...change };
    setSettings(next);
    api
      .saveSettings(next)
      .then((saved) => {
        setSettings(saved);
        setFolder(saved.downloadsFolder);
        setError(null);
      })
      .catch((e) => {
        setError(errorText(e));
        load();
      });
  };

  const saveFolder = (value: string) => {
    if (settings && value.trim() !== settings.downloadsFolder) save({ downloadsFolder: value.trim() });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-neutral-200 bg-white px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900">
        <button
          type="button"
          onClick={onClose}
          className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[13px] font-medium text-neutral-700 hover:bg-neutral-100 dark:text-neutral-200 dark:hover:bg-neutral-800"
        >
          <Icon name="back" className="size-4" />
          Downloads
        </button>
        <h1 className="text-[15px] font-semibold">Settings</h1>
      </header>
      {error && (
        <p role="alert" className="border-b border-red-200 bg-red-50 px-4 py-2 text-[13px] text-red-900 dark:border-red-900 dark:bg-red-950 dark:text-red-100">
          {error}
        </p>
      )}
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Settings sections" className="w-52 shrink-0 space-y-0.5 border-r border-neutral-200 bg-neutral-50 px-2 py-3 dark:border-neutral-800 dark:bg-neutral-900/60">
          {SECTIONS.map(({ id, label }) => (
            <button
              key={id}
              type="button"
              aria-current={section === id ? "page" : undefined}
              onClick={() => setSection(id)}
              className={`block w-full rounded-md px-2.5 py-1.5 text-left text-[13px] ${
                section === id
                  ? "bg-sky-600/10 font-medium text-sky-800 dark:bg-sky-400/15 dark:text-sky-200"
                  : "text-neutral-700 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800"
              }`}
            >
              {label}
            </button>
          ))}
        </nav>
        <main className="min-w-0 flex-1 overflow-y-auto bg-white dark:bg-neutral-950">
          {!settings ? (
            <p className="p-6 text-sm text-neutral-500">Loading…</p>
          ) : section === "general" ? (
            <Panel title="General">
              <Row label="Downloads folder" hint="Where downloads go when their category has no folder of its own.">
                <FolderField
                  id="downloads-folder"
                  value={folder}
                  onChange={(value, picked) => {
                    setFolder(value);
                    if (picked) saveFolder(value);
                  }}
                  onBlur={() => saveFolder(folder)}
                />
              </Row>
              <Toggle
                label="Start OmniDm with Windows"
                hint="It starts in the tray, ready for the browser."
                on={settings.startWithWindows}
                onChange={(on) => save({ startWithWindows: on })}
              />
              <Toggle
                label="Ask before downloading"
                hint="When a download is taken from the browser, show where it will be saved first."
                on={settings.askBeforeDownload}
                onChange={(on) => save({ askBeforeDownload: on })}
              />
              <Toggle
                label="Watch the clipboard for download links"
                hint="Copying a link to a file (one whose type a category knows) opens Add download."
                on={settings.clipboardMonitor}
                onChange={(on) => save({ clipboardMonitor: on })}
              />
              <Toggle
                label="Notify when downloads finish"
                hint="A Windows notification when a download completes or fails."
                on={settings.notifyFinished}
                onChange={(on) => save({ notifyFinished: on })}
              />
            </Panel>
          ) : section === "connections" ? (
            <Panel title="Connections">
              <Row
                label="Downloads at once"
                hint="More wait their turn. Browser downloads always start at once, and queues have their own limits."
              >
                <select
                  aria-label="Downloads at once"
                  value={settings.maxActive}
                  onChange={(e) => save({ maxActive: Number(e.target.value) })}
                  className={`${selectClass} w-24`}
                >
                  {Array.from({ length: 16 }, (_, i) => i + 1).map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </select>
              </Row>
              <Row
                label="Connections per download"
                hint="The most a download uses. It starts with fewer and adds more while they make it faster. Applies to downloads that start from now on."
              >
                <select
                  aria-label="Connections per download"
                  value={settings.maxConnections}
                  onChange={(e) => save({ maxConnections: Number(e.target.value) })}
                  className={`${selectClass} w-24`}
                >
                  {CONNECTIONS.map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </select>
              </Row>
            </Panel>
          ) : (
            <Panel title="Categories">
              <CategoriesEditor
                categories={categories}
                downloadsFolder={settings.downloadsFolder}
                onChanged={onChanged}
              />
            </Panel>
          )}
        </main>
      </div>
    </div>
  );
}

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="max-w-3xl space-y-1 p-6">
      <h2 className="pb-3 text-lg font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="space-y-2 border-b border-neutral-100 py-4 dark:border-neutral-900">
      <div>
        <p className="text-[13px] font-medium">{label}</p>
        {hint && <p className="text-xs text-neutral-500">{hint}</p>}
      </div>
      {children}
    </div>
  );
}

function Toggle({
  label,
  hint,
  on,
  onChange,
}: {
  label: string;
  hint?: string;
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <div className="flex items-center gap-4 border-b border-neutral-100 py-4 dark:border-neutral-900">
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-medium">{label}</p>
        {hint && <p className="text-xs text-neutral-500">{hint}</p>}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={label}
        onClick={() => onChange(!on)}
        className={`relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:ring-2 focus-visible:ring-sky-500/50 focus-visible:outline-none ${
          on ? "bg-sky-600" : "bg-neutral-300 dark:bg-neutral-700"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 size-4 rounded-full bg-white shadow transition-transform ${
            on ? "translate-x-4" : ""
          }`}
        />
      </button>
    </div>
  );
}
