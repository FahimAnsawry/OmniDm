// SPDX-License-Identifier: GPL-3.0-or-later

// The main window: lists in the sidebar, the downloads, and the actions on them.

import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { AfterQueue } from "../../protocol/ts/AfterQueue";
import { AddDialog } from "./components/AddDialog";
import { AfterQueueBanner } from "./components/AfterQueueBanner";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import { DeleteDialog } from "./components/DeleteDialog";
import { DownloadTable } from "./components/DownloadTable";
import { SettingsPage } from "./components/SettingsPage";
import { Icon } from "./components/Icon";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { api, errorText } from "./lib/api";
import {
  type Row,
  type Sort,
  type View,
  canResume,
  displayName,
  inView,
  isActive,
  sortRows,
  withLive,
} from "./lib/downloads";
import { formatSpeed } from "./lib/format";
import { useAppData } from "./lib/useAppData";

function App() {
  const { downloads, live, queues, categories, refresh } = useAppData();
  const [view, setView] = useState<View>({ kind: "status", status: "all" });
  const [sort, setSort] = useState<Sort>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [deleting, setDeleting] = useState<Row[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [afterQueue, setAfterQueue] = useState<AfterQueue | null>(null);
  /** The Add download dialog, with a copied link when the clipboard monitor opened it. */
  const [adding, setAdding] = useState<{ url?: string } | null>(null);
  const [page, setPage] = useState<"downloads" | "settings">("downloads");

  useEffect(() => {
    const stop = listen<string>("clipboard-link", ({ payload }) => {
      setPage("downloads");
      setAdding({ url: payload });
    });
    return () => void stop.then((f) => f());
  }, []);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
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

  const running = rows.filter((r) => r.status === "running");
  const totalSpeed = running.reduce((sum, r) => sum + (r.speed ?? 0), 0);

  return (
    <div className="flex h-screen flex-col bg-white font-sans text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      {afterQueue && <AfterQueueBanner notice={afterQueue} onClose={() => setAfterQueue(null)} />}
      {page === "settings" ? (
        <SettingsPage categories={categories} onChanged={refresh} onClose={() => setPage("downloads")} />
      ) : (
        <>
          <Toolbar
            onAdd={() => setAdding({})}
            onSettings={() => setPage("settings")}
            canResume={chosen.some((r) => canResume(r.status))}
            canPause={chosen.some((r) => isActive(r.status))}
            canDelete={chosen.length > 0}
            onResume={() => resume(chosen)}
            onPause={() => pause(chosen)}
            onDelete={() => setDeleting(chosen)}
            queue={queue}
            onStartQueue={() => queue && run(api.startQueue(queue.name))}
            onStopQueue={() => queue && run(api.stopQueue(queue.name))}
            search={search}
            onSearch={setSearch}
          />
          {error && (
            <div
              role="alert"
              className="flex items-center gap-3 border-b border-red-200 bg-red-50 px-4 py-2 text-[13px] text-red-900 dark:border-red-900 dark:bg-red-950 dark:text-red-100"
            >
              <Icon name="failed" className="size-4" />
              <p className="flex-1">{error}</p>
              <button type="button" aria-label="Dismiss" onClick={() => setError(null)} className="rounded p-0.5 hover:bg-red-100 dark:hover:bg-red-900">
                <Icon name="close" className="size-4" />
              </button>
            </div>
          )}
          <div className="flex min-h-0 flex-1">
            <Sidebar
              rows={rows}
              queues={queues}
              categories={categories}
              view={view}
              onView={(v) => {
                setView(v);
                setSort(null);
              }}
            />
            <main className="flex min-w-0 flex-1 flex-col">
              <DownloadTable
                rows={shown}
                empty={rows.length === 0}
                selected={selected}
                onSelect={setSelected}
                sort={sort}
                onSort={setSort}
                onOpen={open}
                onDelete={() => setDeleting(chosen)}
                onContextMenu={(_row, x, y) => setMenu({ x, y })}
              />
              <footer className="flex items-center gap-4 border-t border-neutral-200 bg-neutral-50 px-4 py-1.5 text-xs text-neutral-500 tabular-nums dark:border-neutral-800 dark:bg-neutral-900">
                <span>
                  {shown.length === rows.length ? `${rows.length} downloads` : `${shown.length} of ${rows.length} downloads`}
                </span>
                {chosen.length > 1 && <span>{chosen.length} selected</span>}
                {running.length > 0 && (
                  <span className="ml-auto">
                    {running.length} downloading · {formatSpeed(totalSpeed)}
                  </span>
                )}
              </footer>
            </main>
          </div>
        </>
      )}
      {menu && chosen.length > 0 && (
        <ContextMenu x={menu.x} y={menu.y} items={menuItems()} onClose={() => setMenu(null)} />
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
