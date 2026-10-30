// SPDX-License-Identifier: GPL-3.0-or-later

// The main window: its title bar, lists in the sidebar, the downloads, and the actions on them.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { AfterQueue } from "../../protocol/ts/AfterQueue";
import { AddDialog } from "./components/AddDialog";
import { AfterQueueBanner } from "./components/AfterQueueBanner";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import { DeleteDialog } from "./components/DeleteDialog";
import { DownloadTable } from "./components/DownloadTable";
import { EmptyState } from "./components/EmptyState";
import { Icon } from "./components/Icon";
import { PanelHeader } from "./components/PanelHeader";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { api, errorText } from "./lib/api";
import {
  type Row,
  type Sort,
  type SortKey,
  type View,
  canResume,
  displayName,
  inView,
  isActive,
  sortRows,
  withLive,
} from "./lib/downloads";
import { formatBytes } from "./lib/format";
import { playViewChange } from "./lib/viewChange";
import { useAppData } from "./lib/useAppData";

/** Speed samples kept for the sidebar's graph, one a second. */
const SPEED_SAMPLES = 40;

const STATUS_TITLE: Record<Extract<View, { kind: "status" }>["status"], string> = {
  all: "All downloads",
  active: "Downloading",
  paused: "Paused",
  completed: "Completed",
  failed: "Failed",
};

const SORTS: { key: SortKey; label: string }[] = [
  { key: "name", label: "Name" },
  { key: "size", label: "Size" },
  { key: "progress", label: "Progress" },
  { key: "speed", label: "Speed" },
  { key: "left", label: "Time left" },
  { key: "status", label: "Status" },
  { key: "added", label: "Added" },
];

/** Typing in a field, where Ctrl+V pastes text rather than a new download. */
const typing = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));

function App() {
  const { downloads, live, queues, categories, refresh } = useAppData();
  const [view, setView] = useState<View>({ kind: "status", status: "all" });
  const [sort, setSort] = useState<Sort>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [sortMenu, setSortMenu] = useState<{ x: number; y: number } | null>(null);
  const [deleting, setDeleting] = useState<Row[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [afterQueue, setAfterQueue] = useState<AfterQueue | null>(null);
  /** The Add download dialog, with a copied link when the clipboard monitor opened it. */
  const [adding, setAdding] = useState<{ url?: string } | null>(null);
  const [page, setPage] = useState<"downloads" | "settings">("downloads");
  const [askFirst, setAskFirst] = useState<boolean | null>(null);
  const [speeds, setSpeeds] = useState<number[]>([]);
  const searchInput = useRef<HTMLInputElement>(null);
  const panel = useRef<HTMLElement>(null);
  /** The list on show, to play the change when another one is picked. */
  const viewKey = JSON.stringify(view);
  const shownView = useRef(viewKey);
  useLayoutEffect(() => {
    if (shownView.current === viewKey) return;
    shownView.current = viewKey;
    playViewChange(panel.current);
  }, [viewKey]);

  useEffect(() => {
    const stop = listen<string>("clipboard-link", ({ payload }) => {
      setPage("downloads");
      setAdding({ url: payload });
    });
    return () => void stop.then((f) => f());
  }, []);

  useEffect(() => {
    const load = () =>
      api
        .getSettings()
        .then((s) => setAskFirst(s.askBeforeDownload))
        .catch(() => {});
    load();
    const stop = listen("settings-changed", load);
    return () => void stop.then((f) => f());
  }, []);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      const letter = event.key.toLowerCase();
      if (letter === "n") {
        event.preventDefault();
        setAdding((current) => current ?? {});
      } else if (letter === "k") {
        event.preventDefault();
        setPage("downloads");
        searchInput.current?.focus();
        searchInput.current?.select();
      } else if (letter === "v" && !typing(event.target) && !document.querySelector("dialog[open]")) {
        // The Add download dialog picks up a copied link by itself.
        event.preventDefault();
        setAdding((current) => current ?? {});
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  useEffect(() => {
    // Shown while counting down, and when it failed; gone once done or cancelled.
    const stop = listen<AfterQueue>("after-queue", ({ payload }) =>
      setAfterQueue(payload.outcome == null || payload.outcome === "failed" ? payload : null),
    );
    return () => void stop.then((f) => f());
  }, []);

  const rows = useMemo(() => downloads.map((d) => withLive(d, live.get(d.downloadId))), [downloads, live]);
  const queue = view.kind === "queue" ? queues.find((q) => q.name === view.name) : undefined;
  const shown = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const matching = rows.filter(
      (row) =>
        inView(row, view) &&
        (!needle || displayName(row).toLowerCase().includes(needle) || row.url.toLowerCase().includes(needle)),
    );
    return sortRows(matching, sort, queue);
  }, [rows, view, search, sort, queue]);

  // Selection follows what's on screen: rows that leave the view are deselected.
  useEffect(() => {
    setSelected((current) => {
      const visible = new Set(shown.map((r) => r.downloadId));
      const kept = new Set([...current].filter((id) => visible.has(id)));
      return kept.size === current.size ? current : kept;
    });
  }, [shown]);

  const chosen = useMemo(() => shown.filter((r) => selected.has(r.downloadId)), [shown, selected]);

  const running = rows.filter((r) => r.status === "running");
  const totalSpeed = running.reduce((sum, r) => sum + (r.speed ?? 0), 0);

  // A sample of the total speed every second, for the sidebar's graph.
  const speedNow = useRef(0);
  speedNow.current = running.length ? totalSpeed : -1;
  useEffect(() => {
    const timer = window.setInterval(() => {
      const now = speedNow.current;
      setSpeeds((list) => (now < 0 ? (list.length ? [] : list) : [...list, now].slice(-SPEED_SAMPLES)));
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const run = useCallback(
    (action: Promise<unknown>) =>
      action
        .then(() => setError(null))
        .catch((e) => setError(errorText(e)))
        .finally(refresh),
    [refresh],
  );

  const resume = (targets: Row[]) =>
    run(api.resume(targets.filter((r) => canResume(r.status)).map((r) => r.downloadId)));
  const pause = (targets: Row[]) =>
    run(api.pause(targets.filter((r) => isActive(r.status)).map((r) => r.downloadId)));
  /** Double-click or Enter: open a finished file, continue a stopped one, or watch a running one. */
  const open = (row: Row) => {
    if (row.status === "completed") run(api.openFile(row.downloadId));
    else if (canResume(row.status)) resume([row]);
    else run(api.openDetails(row.downloadId));
  };

  const menuItems = (): MenuItem[] => {
    const one = chosen.length === 1 ? chosen[0] : undefined;
    return [
      {
        label: "Open",
        icon: "open",
        disabled: one?.status !== "completed",
        onSelect: () => one && run(api.openFile(one.downloadId)),
      },
      {
        label: "Show in folder",
        icon: "folder",
        disabled: !one,
        onSelect: () => one && run(api.showInFolder(one.downloadId)),
      },
      {
        label: "Details",
        icon: "details",
        disabled: !one,
        onSelect: () => one && run(api.openDetails(one.downloadId)),
      },
      "separator",
      { label: "Resume", icon: "play", disabled: !chosen.some((r) => canResume(r.status)), onSelect: () => resume(chosen) },
      { label: "Pause", icon: "pause", disabled: !chosen.some((r) => isActive(r.status)), onSelect: () => pause(chosen) },
      "separator",
      {
        label: chosen.length > 1 ? "Copy links" : "Copy link",
        icon: "link",
        onSelect: () => run(navigator.clipboard.writeText(chosen.map((r) => r.url).join("\n"))),
      },
      "separator",
      { label: "Delete…", icon: "trash", danger: true, onSelect: () => setDeleting(chosen) },
    ];
  };

  const sortItems = (): MenuItem[] => [
    { label: "Newest first", icon: sort ? undefined : "check", onSelect: () => setSort(null) },
    "separator",
    ...SORTS.map(({ key, label }): MenuItem => {
      const current = sort?.key === key ? sort : null;
      return {
        label: current ? `${label} (${current.descending ? "high to low" : "low to high"})` : label,
        icon: current ? (current.descending ? "arrow-down" : "arrow-up") : undefined,
        onSelect: () => setSort({ key, descending: current ? !current.descending : key === "added" }),
      };
    }),
  ];

  const empty = rows.length === 0;
  const title =
    view.kind === "status" ? STATUS_TITLE[view.status] : view.name;
  const onDisk = shown.reduce((sum, r) => sum + (r.status === "completed" ? (r.total ?? r.downloaded) : r.downloaded), 0);
  const subtitle = empty
    ? "Nothing here yet"
    : [
        `${shown.length} ${shown.length === 1 ? "download" : "downloads"}`,
        `${formatBytes(onDisk)} on disk`,
        queue?.nextStart && !queue.running ? `starts ${queue.nextStart}` : null,
      ]
        .filter(Boolean)
        .join(" · ");

  return (
    <div className="glow-main flex h-full flex-col font-sans text-[#f4f6fb] select-none">
      <TitleBar
        search={search}
        onSearch={(text) => {
          setSearch(text);
          setPage("downloads");
        }}
        searchInput={searchInput}
        onSettings={() => setPage((current) => (current === "settings" ? "downloads" : "settings"))}
        settingsOpen={page === "settings"}
      />
      {afterQueue && <AfterQueueBanner notice={afterQueue} onClose={() => setAfterQueue(null)} />}
      {error && (
        <div
          role="alert"
          className="mx-2.5 mb-2 flex items-center gap-3 rounded-xl border border-[#e5484d4d] bg-[#e5484d1f] px-4 py-2 text-[13px] text-[#ffc9cb]"
        >
          <Icon name="failed" className="size-4" />
          <p className="flex-1 select-text">{error}</p>
          <button type="button" aria-label="Dismiss" onClick={() => setError(null)} className="rounded p-0.5 text-[#ff8a8f] hover:bg-[#e5484d]/[0.16] hover:text-[#ffc9cb]">
            <Icon name="close" className="size-4" />
          </button>
        </div>
      )}
      {page === "settings" ? (
        <div className="flex min-h-0 flex-1 pr-2.5 pb-2.5">
          <SettingsPage categories={categories} onChanged={refresh} onClose={() => setPage("downloads")} />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 pr-2.5 pb-2.5">
          <Sidebar
            rows={rows}
            queues={queues}
            categories={categories}
            view={view}
            onView={(v) => {
              setView(v);
              setSort(null);
            }}
            speed={totalSpeed}
            speeds={speeds}
          />
          <main
            ref={panel}
            className="flex min-w-0 flex-1 flex-col rounded-2xl border border-white/[0.08] bg-white/[0.024] shadow-[0_24px_48px_-16px_rgb(0_0_0/0.5)]"
          >
            <PanelHeader
              title={title}
              subtitle={subtitle}
              empty={empty}
              canResume={chosen.some((r) => canResume(r.status))}
              canPause={chosen.some((r) => isActive(r.status))}
              canDelete={chosen.length > 0}
              onResume={() => resume(chosen)}
              onPause={() => pause(chosen)}
              onDelete={() => setDeleting(chosen)}
              queue={queue}
              onStartQueue={() => queue && run(api.startQueue(queue.name))}
              onStopQueue={() => queue && run(api.stopQueue(queue.name))}
              onSortMenu={(x, y) => setSortMenu({ x, y })}
              onAdd={() => setAdding({})}
            />
            {empty ? (
              <>
                <div className="h-px bg-white/[0.05]" />
                <EmptyState onPaste={() => setAdding({})} onExtension={() => run(api.openExtensionGuide())} />
              </>
            ) : (
              <DownloadTable
                rows={shown}
                selected={selected}
                onSelect={setSelected}
                sort={sort}
                onSort={setSort}
                onOpen={open}
                onRetry={(row) => resume([row])}
                onDelete={() => setDeleting(chosen)}
                onContextMenu={(_row, x, y) => setMenu({ x, y })}
              />
            )}
            <footer role="status" className="flex h-10 shrink-0 items-center gap-3 px-6 text-xs tabular-nums">
              <span className="text-[#8b93a9]">
                {shown.length === rows.length
                  ? `${rows.length} ${rows.length === 1 ? "download" : "downloads"}`
                  : `${shown.length} of ${rows.length} downloads`}
                {chosen.length > 0 && ` · ${chosen.length} selected`}
              </span>
              {askFirst != null && (
                <label className="ml-auto flex cursor-pointer items-center gap-3 font-medium text-[#c9cfdd]">
                  Ask before downloading
                  <input
                    type="checkbox"
                    role="switch"
                    checked={askFirst}
                    onChange={(e) => {
                      const ask = e.target.checked;
                      setAskFirst(ask);
                      run(api.setAskBeforeDownload(ask));
                    }}
                    className="relative h-[18px] w-8 cursor-pointer appearance-none rounded-full bg-white/[0.12] transition-colors before:absolute before:top-0.5 before:left-0.5 before:size-3.5 before:rounded-full before:bg-white before:transition-transform checked:bg-[linear-gradient(90deg,#4a6cff,#7c9bff)] checked:before:translate-x-3.5 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#6b88ff]"
                  />
                </label>
              )}
            </footer>
          </main>
        </div>
      )}
      {menu && chosen.length > 0 && (
        <ContextMenu x={menu.x} y={menu.y} items={menuItems()} onClose={() => setMenu(null)} />
      )}
      {sortMenu && (
        <ContextMenu x={sortMenu.x} y={sortMenu.y} items={sortItems()} onClose={() => setSortMenu(null)} />
      )}
      {adding && (
        <AddDialog
          key={adding.url ?? ""}
          queues={queues}
          initialUrl={adding.url}
          onClose={() => {
            setAdding(null);
            refresh();
          }}
        />
      )}
      {deleting && deleting.length > 0 && (
        <DeleteDialog
          names={deleting.map(displayName)}
          onCancel={() => setDeleting(null)}
          onConfirm={(deleteFiles) => {
            const ids = deleting.map((r) => r.downloadId);
            setDeleting(null);
            setSelected(new Set());
            run(api.remove(ids, deleteFiles));
          }}
        />
      )}
    </div>
  );
}

export default App;
