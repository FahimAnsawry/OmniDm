// SPDX-License-Identifier: GPL-3.0-or-later

// The download list. Only the rows in view are drawn, so long lists stay smooth.

import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent } from "react";
import {
  type Row,
  type Sort,
  type SortKey,
  STATUS_LABEL,
  displayName,
  isActive,
  secondsLeft,
} from "../lib/downloads";
import { formatAdded, formatBytes, formatDuration, formatFullDate, formatSpeed } from "../lib/format";
import { Icon } from "./Icon";

const ROW_HEIGHT = 34;
/** Rows drawn beyond each edge of the view, so fast scrolling doesn't show gaps. */
const OVERSCAN = 8;
const COLUMNS = "grid-cols-[minmax(12rem,1fr)_5.5rem_8.5rem_5.5rem_4.75rem_6.5rem_5.5rem]";

const HEADERS: { key: SortKey; label: string; align?: "right" }[] = [
  { key: "name", label: "Name" },
  { key: "size", label: "Size", align: "right" },
  { key: "progress", label: "Progress" },
  { key: "speed", label: "Speed", align: "right" },
  { key: "left", label: "Time left", align: "right" },
  { key: "status", label: "Status" },
  { key: "added", label: "Added", align: "right" },
];

type Props = {
  rows: Row[];
  /** No downloads at all, as opposed to none in this view. */
  empty: boolean;
  selected: Set<number>;
  onSelect: (ids: Set<number>) => void;
  sort: Sort;
  onSort: (sort: Sort) => void;
  onOpen: (row: Row) => void;
  onDelete: () => void;
  onContextMenu: (row: Row, x: number, y: number) => void;
};

export function DownloadTable(props: Props) {
  const { rows, selected, onSelect } = props;
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(600);
  /** Where a shift-click or shift-arrow selection starts. */
  const anchor = useRef<number | null>(null);
  /** The row the keyboard is on. */
  const [cursor, setCursor] = useState<number | null>(null);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const indexOf = (id: number | null) => (id == null ? -1 : rows.findIndex((r) => r.downloadId === id));

  // Keep the keyboard's row in view.
  useEffect(() => {
    const element = scroller.current;
    const index = indexOf(cursor);
    if (!element || index < 0) return;
    const top = index * ROW_HEIGHT;
    const header = ROW_HEIGHT;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + ROW_HEIGHT > element.scrollTop + element.clientHeight - header)
      element.scrollTop = top + ROW_HEIGHT - element.clientHeight + header;
    // Only when the cursor moves, not when rows update.
  }, [cursor]);

  const selectRange = (from: number, to: number, add: boolean) => {
    const [lo, hi] = from < to ? [from, to] : [to, from];
    const next = new Set(add ? selected : []);
    for (let i = lo; i <= hi; i += 1) next.add(rows[i].downloadId);
    onSelect(next);
  };

  const click = (event: MouseEvent, index: number) => {
    const id = rows[index].downloadId;
    setCursor(id);
    if (event.shiftKey && anchor.current != null) {
      const from = indexOf(anchor.current);
      if (from >= 0) {
        selectRange(from, index, event.ctrlKey);
        return;
      }
    }
    anchor.current = id;
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      onSelect(next);
    } else {
      onSelect(new Set([id]));
    }
  };

  const keyDown = (event: KeyboardEvent) => {
    if (rows.length === 0) return;
    const current = indexOf(cursor);
    const move = (to: number) => {
      event.preventDefault();
      const index = Math.max(0, Math.min(rows.length - 1, to));
      const id = rows[index].downloadId;
      setCursor(id);
      if (event.shiftKey && anchor.current != null && indexOf(anchor.current) >= 0) {
        selectRange(indexOf(anchor.current), index, false);
      } else {
        anchor.current = id;
        onSelect(new Set([id]));
      }
    };
    const page = Math.max(1, Math.floor(height / ROW_HEIGHT) - 2);
    switch (event.key) {
      case "ArrowDown":
        return move(current < 0 ? 0 : current + 1);
      case "ArrowUp":
        return move(current < 0 ? 0 : current - 1);
      case "PageDown":
        return move(current + page);
      case "PageUp":
        return move(current - page);
      case "Home":
        return move(0);
      case "End":
        return move(rows.length - 1);
      case "Delete":
        if (selected.size > 0) {
          event.preventDefault();
          props.onDelete();
        }
        return;
      case "Enter":
        if (current >= 0) {
          event.preventDefault();
          props.onOpen(rows[current]);
        }
        return;
      case "Escape":
        onSelect(new Set());
        return;
      case "a":
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault();
          onSelect(new Set(rows.map((r) => r.downloadId)));
        }
        return;
    }
  };

  const sortBy = (key: SortKey) => {
    const { sort } = props;
    // Ascending, then descending, then back to the view's own order.
    if (!sort || sort.key !== key) props.onSort({ key, descending: key === "added" });
    else if (sort.descending === (key === "added")) props.onSort({ key, descending: !sort.descending });
    else props.onSort(null);
  };

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(rows.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
  const cursorIndex = indexOf(cursor);

  return (
    <div
      ref={scroller}
      role="grid"
      aria-label="Downloads"
      aria-rowcount={rows.length + 1}
      aria-multiselectable="true"
      tabIndex={0}
      onKeyDown={keyDown}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      className="relative min-h-0 flex-1 overflow-auto bg-white outline-none focus-visible:ring-2 focus-visible:ring-sky-500/40 focus-visible:ring-inset dark:bg-neutral-950"
    >
      <div className="min-w-[46rem]">
        <div
          role="row"
          aria-rowindex={1}
          className={`sticky top-0 z-10 grid ${COLUMNS} h-[34px] items-center border-b border-neutral-200 bg-neutral-50/95 text-[11px] font-semibold tracking-wide text-neutral-500 uppercase backdrop-blur dark:border-neutral-800 dark:bg-neutral-900/95`}
        >
          {HEADERS.map(({ key, label, align }) => {
            const sorted = props.sort?.key === key ? props.sort : null;
            return (
              <div
                key={key}
                role="columnheader"
                aria-sort={sorted ? (sorted.descending ? "descending" : "ascending") : undefined}
              >
                <button
                  type="button"
                  onClick={() => sortBy(key)}
                  className={`flex h-[34px] w-full items-center gap-1 px-3 whitespace-nowrap uppercase hover:text-neutral-800 dark:hover:text-neutral-200 ${
                    align === "right" ? "justify-end" : ""
                  }`}
                >
                  {label}
                  {sorted && <Icon name={sorted.descending ? "down" : "up"} className="size-3" />}
                </button>
              </div>
            );
          })}
        </div>
        {rows.length === 0 ? (
          <p className="px-6 py-16 text-center text-sm text-neutral-500">
            {props.empty
              ? "No downloads yet. Click a download link in your browser and it appears here."
              : "Nothing here."}
          </p>
        ) : (
          <div style={{ height: rows.length * ROW_HEIGHT }} className="relative">
            {rows.slice(first, last).map((row, offset) => {
              const index = first + offset;
              return (
                <DownloadRow
                  key={row.downloadId}
                  row={row}
                  index={index}
                  selected={selected.has(row.downloadId)}
                  cursor={index === cursorIndex}
                  onClick={click}
                  onDoubleClick={props.onOpen}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    if (!selected.has(row.downloadId)) {
                      anchor.current = row.downloadId;
                      setCursor(row.downloadId);
                      onSelect(new Set([row.downloadId]));
                    }
                    props.onContextMenu(row, event.clientX, event.clientY);
                  }}
                />
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

const STATUS_STYLE: Record<string, string> = {
  running: "text-sky-700 dark:text-sky-400",
  probing: "text-sky-700 dark:text-sky-400",
  completed: "text-emerald-700 dark:text-emerald-400",
  failed: "text-red-700 dark:text-red-400",
  paused: "text-amber-700 dark:text-amber-400",
};

type RowProps = {
  row: Row;
  index: number;
  selected: boolean;
  cursor: boolean;
  onClick: (event: MouseEvent, index: number) => void;
  onDoubleClick: (row: Row) => void;
  onContextMenu: (event: MouseEvent) => void;
};

const DownloadRow = memo(function DownloadRow({
  row,
  index,
  selected,
  cursor,
  onClick,
  onDoubleClick,
  onContextMenu,
}: RowProps) {
  const done = row.status === "completed";
  const fraction = done ? 1 : row.total ? Math.min(1, row.downloaded / row.total) : 0;
  const left = secondsLeft(row);
  const running = row.status === "running";
  const bar = done
    ? "bg-emerald-500"
    : row.status === "failed"
      ? "bg-red-500"
      : running
        ? "bg-sky-500"
        : "bg-neutral-400 dark:bg-neutral-500";
  return (
    <div
      role="row"
      aria-rowindex={index + 2}
      aria-selected={selected}
      onMouseDown={(e) => {
        // Keep text from being selected by shift-clicks.
        if (e.shiftKey) e.preventDefault();
      }}
      onClick={(e) => onClick(e, index)}
      onDoubleClick={() => onDoubleClick(row)}
      onContextMenu={onContextMenu}
      style={{ transform: `translateY(${index * ROW_HEIGHT}px)` }}
      className={`absolute inset-x-0 top-0 grid ${COLUMNS} h-[34px] cursor-default items-center border-b border-neutral-100 text-[13px] select-none dark:border-neutral-900 ${
        selected
          ? "bg-sky-100 dark:bg-sky-900/40"
          : "hover:bg-neutral-50 dark:hover:bg-neutral-900"
      } ${cursor ? "outline outline-1 -outline-offset-1 outline-sky-400/70" : ""}`}
    >
      <div role="gridcell" className="truncate px-3 font-medium" title={`${displayName(row)}\n${row.url}`}>
        {displayName(row)}
      </div>
      <div role="gridcell" className="px-3 text-right text-neutral-600 tabular-nums dark:text-neutral-400">
        {row.total != null ? formatBytes(row.total) : ""}
      </div>
      <div role="gridcell" className="flex items-center gap-2 px-3">
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800">
          <div className={`h-full rounded-full ${bar}`} style={{ width: `${fraction * 100}%` }} />
        </div>
        <span className="w-9 text-right text-xs text-neutral-500 tabular-nums">
          {row.total || done ? `${Math.floor(fraction * 100)}%` : ""}
        </span>
      </div>
      <div role="gridcell" className="px-3 text-right text-neutral-600 tabular-nums dark:text-neutral-400">
        {running && row.speed != null ? formatSpeed(row.speed) : ""}
      </div>
      <div role="gridcell" className="px-3 text-right text-neutral-600 tabular-nums dark:text-neutral-400">
        {Number.isFinite(left) ? formatDuration(left) : ""}
      </div>
      <div
        role="gridcell"
        title={row.error}
        className={`truncate px-3 ${STATUS_STYLE[row.status] ?? "text-neutral-600 dark:text-neutral-400"}`}
      >
        {STATUS_LABEL[row.status] ?? row.status}
        {row.error && !isActive(row.status) && <span className="sr-only">: {row.error}</span>}
      </div>
      <div
        role="gridcell"
        title={formatFullDate(row.addedAt)}
        className="px-3 text-right text-neutral-500 tabular-nums"
      >
        {formatAdded(row.addedAt)}
      </div>
    </div>
  );
});
