// SPDX-License-Identifier: GPL-3.0-or-later

// The download list as the window sees it: stored summaries plus live progress.

import type { DownloadSummary } from "../../../protocol/ts/DownloadSummary";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";

/** Live numbers from the latest progress update of a running download. */
export type Live = { downloaded: number; total?: number; speed: number; connections: number };

export type Row = DownloadSummary & { speed?: number; connections?: number };

export function withLive(row: DownloadSummary, live: Live | undefined): Row {
  if (!live) return row;
  return {
    ...row,
    status: row.status === "completed" ? row.status : "running",
    downloaded: live.downloaded,
    total: live.total ?? row.total,
    speed: live.speed,
    connections: live.connections,
  };
}

export const isActive = (status: string) =>
  status === "running" || status === "queued" || status === "probing";
export const canResume = (status: string) =>
  status === "paused" || status === "failed" || status === "cancelled";

export function displayName(row: DownloadSummary): string {
  if (row.fileName) return row.fileName;
  try {
    const url = new URL(row.url);
    const last = url.pathname.split("/").filter(Boolean).pop();
    return last ? decodeURIComponent(last) : url.host;
  } catch {
    return row.url;
  }
}

export const STATUS_LABEL: Record<string, string> = {
  asking: "Asking you",
  queued: "Waiting",
  probing: "Starting",
  running: "Downloading",
  paused: "Paused",
  completed: "Complete",
  failed: "Failed",
  cancelled: "Cancelled",
};

/** What the sidebar can show. */
export type View =
  | { kind: "status"; status: "all" | "active" | "paused" | "completed" | "failed" }
  | { kind: "category"; name: string }
  | { kind: "queue"; name: string };

export function sameView(a: View, b: View): boolean {
  if (a.kind !== b.kind) return false;
  return a.kind === "status"
    ? a.status === (b as typeof a).status
    : a.name === (b as typeof a).name;
}

export function inView(row: Row, view: View): boolean {
  switch (view.kind) {
    case "status":
      switch (view.status) {
        case "all":
          return true;
        case "active":
          return isActive(row.status);
        case "paused":
          return row.status === "paused" || row.status === "cancelled";
        default:
          return row.status === view.status;
      }
    case "category":
      return row.category === view.name;
    case "queue":
      return row.queue === view.name;
  }
}

export type SortKey = "name" | "size" | "progress" | "speed" | "left" | "status" | "added";
export type Sort = { key: SortKey; descending: boolean } | null;

const progressOf = (row: Row) =>
  row.status === "completed" ? 1 : row.total ? row.downloaded / row.total : 0;

/** Seconds left, or `Infinity` when unknown. */
export function secondsLeft(row: Row): number {
  if (row.status !== "running" || !row.speed || row.total == null) return Infinity;
  return Math.max(0, row.total - row.downloaded) / row.speed;
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/**
 * Sorts by `sort`; with none, a queue's view keeps the queue's order and every other view
 * puts the newest first.
 */
export function sortRows(rows: Row[], sort: Sort, queue?: QueueSummary): Row[] {
  const sorted = [...rows];
  if (!sort) {
    if (queue) {
      const place = new Map(queue.downloadIds.map((id, index) => [id, index]));
      sorted.sort((a, b) => (place.get(a.downloadId) ?? 0) - (place.get(b.downloadId) ?? 0));
    } else {
      sorted.sort((a, b) => b.addedAt - a.addedAt || b.downloadId - a.downloadId);
    }
    return sorted;
  }
  const compare: Record<SortKey, (a: Row, b: Row) => number> = {
    name: (a, b) => collator.compare(displayName(a), displayName(b)),
    size: (a, b) => (a.total ?? -1) - (b.total ?? -1),
    progress: (a, b) => progressOf(a) - progressOf(b),
    speed: (a, b) => (a.speed ?? 0) - (b.speed ?? 0),
    left: (a, b) => {
      const [x, y] = [secondsLeft(a), secondsLeft(b)];
      return x === y ? 0 : x < y ? -1 : 1;
    },
    status: (a, b) => collator.compare(STATUS_LABEL[a.status] ?? a.status, STATUS_LABEL[b.status] ?? b.status),
    added: (a, b) => a.addedAt - b.addedAt || a.downloadId - b.downloadId,
  };
  const by = compare[sort.key];
  sorted.sort((a, b) => (sort.descending ? by(b, a) : by(a, b)));
  return sorted;
}
