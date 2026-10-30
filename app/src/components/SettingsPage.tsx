// SPDX-License-Identifier: GPL-3.0-or-later

// Settings: each change is saved and applied at once. Its menu takes the sidebar's place.

import { type ReactNode, useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import { type AppSettings, api, errorText } from "../lib/api";
import { CategoriesEditor } from "./CategoriesEditor";
import { FolderField, selectClass } from "./Fields";
import { Icon, type IconName } from "./Icon";

type Section = "general" | "connections" | "categories";

const SECTIONS: { id: Section; label: string; icon: IconName; about: string }[] = [
  { id: "general", label: "General", icon: "sliders", about: "Where files go, how OmniDm starts, and when it speaks up." },
  {
    id: "connections",
    label: "Connections",
    icon: "network",
    about: "How many downloads run together, and how many parts each one fetches at a time.",
  },
  {
    id: "categories",
    label: "Categories",
    icon: "tags",
    about: "Sort downloads into folders by file type or site. The first category that matches is used, from the top.",
  },
];

const CONNECTIONS = [1, 2, 4, 6, 8, 12, 16, 24, 32];

/** How far each part of the "How it works" picture has come. */
const PARTS = [100, 90, 76, 70, 58, 44, 30, 12];

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
  const [version, setVersion] = useState("");

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
    getVersion()
      .then(setVersion)
      .catch(() => {});
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

  const current = SECTIONS.find((s) => s.id === section)!;

  return (
    <>
      <nav aria-label="Settings sections" className="flex w-[232px] shrink-0 flex-col gap-0.5 px-3 pt-1 pb-3">
        <button
          type="button"
          onClick={onClose}
          className="flex h-[34px] items-center gap-2.5 rounded-[9px] px-2.5 text-[13px] font-medium text-[#c9cfdd] hover:bg-white/[0.05] hover:text-[#f4f6fb]"
        >
          <Icon name="back" className="text-[#8b93a9]" />
          Downloads
        </button>
        <p className="mt-4 mb-1.5 px-2.5 text-xs font-semibold tracking-[0.08em] text-[#6b7390]">SETTINGS</p>
        {SECTIONS.map(({ id, label, icon }) => {
          const selected = section === id;
          return (
            <button
              key={id}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => setSection(id)}
              className={`flex h-[34px] items-center gap-2.5 rounded-[9px] border px-2.5 text-left text-[13px] ${
                selected
                  ? "border-white/10 bg-white/[0.06] font-semibold text-[#f4f6fb]"
                  : "border-transparent font-medium text-[#c9cfdd] hover:bg-white/[0.04] hover:text-[#f4f6fb]"
              }`}
            >
              <Icon name={icon} className={selected ? "text-[#8ea4ff]" : "text-[#8b93a9]"} />
              {label}
            </button>
          );
        })}
        {version && <p className="mt-auto px-2.5 text-xs text-[#6b7390]">OmniDm {version}</p>}
      </nav>

      <main className="flex min-w-0 flex-1 flex-col rounded-2xl border border-white/[0.08] bg-white/[0.024] shadow-[0_24px_48px_-16px_rgb(0_0_0/0.5)]">
        <header className="px-6 pt-[18px] pb-4">
          <h1 className="text-[22px] font-semibold tracking-[-0.02em] text-[#f4f6fb]">{current.label}</h1>
          <p className="mt-1 text-xs text-[#8b93a9]">{current.about}</p>
        </header>
        {error && (
          <p
            role="alert"
            className="mx-6 mb-4 flex items-center gap-2.5 rounded-xl border border-[#e5484d4d] bg-[#e5484d1f] px-4 py-2 text-[13px] text-[#ffc9cb] select-text"
          >
            <Icon name="failed" className="shrink-0" />
            {error}
          </p>
        )}
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 pb-6">
          {!settings ? (
            <p className="text-[13px] text-[#8b93a9]">Loading…</p>
          ) : section === "general" ? (
            <div className="space-y-5">
              <Group title="Storage">
                <Row label="Downloads folder" hint="Where downloads go when their category has no folder of its own.">
                  <FolderField
                    id="downloads-folder"
                    value={folder}
                    onChange={(value, picked) => {
                      setFolder(value);
                      if (picked) saveFolder(value);
                    }}
                    onBlur={() => saveFolder(folder)}
                    className="w-[320px]"
                  />
                </Row>
              </Group>
              <Group title="Behaviour">
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
              </Group>
              <Group title="Notifications">
                <Toggle
                  label="Notify when downloads finish"
                  hint="A Windows notification when a download completes or fails."
                  on={settings.notifyFinished}
                  onChange={(on) => save({ notifyFinished: on })}
                />
              </Group>
            </div>
          ) : section === "connections" ? (
            <div className="space-y-5">
              <Group title="Limits">
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
              </Group>
              <Group title="How it works">
                <div className="space-y-3 p-4">
                  <div aria-hidden="true" className="flex gap-1">
                    {PARTS.map((done, i) => (
                      <div key={i} className="h-2 flex-1 rounded-full bg-white/[0.08]">
                        <div className="bar-fill h-full rounded-full" style={{ width: `${done}%` }} />
                      </div>
                    ))}
                  </div>
                  <p className="text-xs leading-relaxed text-[#8b93a9]">
                    A file is split into parts that download side by side, one connection each. OmniDm opens a new
                    connection only while it makes the download faster, up to the limit above.
                  </p>
                </div>
              </Group>
            </div>
          ) : (
            <CategoriesEditor categories={categories} downloadsFolder={settings.downloadsFolder} onChanged={onChanged} />
          )}
          {settings && section !== "categories" && (
            <p className="mt-auto flex items-center gap-2 pt-6 text-xs text-[#8b93a9]">
              <span className="size-1.5 rounded-full bg-[#2fd3a0]" />
              Changes are saved as you make them
            </p>
          )}
        </div>
      </main>
    </>
  );
}

/** A titled card of setting rows. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h2 className="text-xs font-semibold tracking-[0.08em] text-[#6b7390] uppercase">{title}</h2>
      <div className="divide-y divide-white/[0.05] rounded-xl border border-white/[0.08] bg-white/[0.02]">{children}</div>
    </section>
  );
}

/** A setting's name and hint, with its control on the right. */
function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-6 px-4 py-3.5">
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-semibold text-[#f4f6fb]">{label}</p>
        {hint && <p className="mt-0.5 text-xs text-[#8b93a9]">{hint}</p>}
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
    <Row label={label} hint={hint}>
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={label}
        onClick={() => onChange(!on)}
        className={`relative h-[18px] w-8 shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#6b88ff] ${
          on ? "bg-[linear-gradient(90deg,#4a6cff,#7c9bff)]" : "bg-white/[0.12]"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 size-3.5 rounded-full transition-transform ${
            on ? "translate-x-3.5 bg-white" : "bg-[#c9cfdd]"
          }`}
        />
      </button>
    </Row>
  );
}
