// SPDX-License-Identifier: GPL-3.0-or-later

// Keeps the downloads, queues and categories in step with the app.

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { DownloadSummary } from "../../../protocol/ts/DownloadSummary";
import type { Finished } from "../../../protocol/ts/Finished";
import type { Progress } from "../../../protocol/ts/Progress";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { api } from "./api";
import type { Live } from "./downloads";

export function useAppData() {
  const [downloads, setDownloads] = useState<DownloadSummary[]>([]);
  const [live, setLive] = useState<Map<number, Live>>(new Map());
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [categories, setCategories] = useState<CategorySummary[]>([]);
  const known = useRef(new Set<number>());
  const pending = useRef<number | null>(null);

  const reload = useCallback(async () => {
    const [list, queueList, categoryList] = await Promise.all([
      api.listDownloads(),
      api.listQueues(),
      api.listCategories(),
    ]);
    known.current = new Set(list.map((d) => d.downloadId));
    setDownloads(list);
    setQueues(queueList);
    setCategories(categoryList);
  }, []);

  /** Reloads soon, once for a burst of changes. */
  const refresh = useCallback(() => {
    if (pending.current != null) return;
    pending.current = window.setTimeout(() => {
      pending.current = null;
      reload().catch((error) => console.error(error));
    }, 100);
  }, [reload]);

  useEffect(() => {
    reload().catch((error) => console.error(error));
    const unlisten = [
      // Every running download's latest numbers, 5 times a second.
      listen<Progress[]>("download-progress", ({ payload }) => {
        if (payload.some((p) => !known.current.has(p.downloadId))) refresh();
        setLive((current) => {
          const next = new Map(current);
          for (const p of payload) {
            next.set(p.downloadId, {
              downloaded: p.downloaded,
              total: p.total,
              speed: p.speed,
              connections: p.connections,
            });
          }
          return next;
        });
      }),
      listen<Finished>("download-finished", ({ payload }) => {
        setLive((current) => {
          if (!current.has(payload.downloadId)) return current;
          const next = new Map(current);
          next.delete(payload.downloadId);
          return next;
        });
        refresh();
      }),
      listen("queue-finished", () => refresh()),
      // Added, answered or cancelled elsewhere (a capture prompt, the Add dialog).
      listen("downloads-changed", () => refresh()),
    ];
    // Things the window wasn't told about (a CLI command) show up when it is looked at.
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      unlisten.forEach((promise) => promise.then((stop) => stop()));
    };
  }, [reload, refresh]);

  return { downloads, live, queues, categories, refresh };
}
