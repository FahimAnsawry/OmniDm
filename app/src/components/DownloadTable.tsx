// SPDX-License-Identifier: GPL-3.0-or-later

// The download list. Only the rows in view are drawn, so long lists stay smooth.

import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
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
import { FileIcon } from "./Chrome";
import { Icon, type IconName } from "./Icon";

/** A row and the gap under it. */
const ROW_HEIGHT = 50;
/** Space above the first row. */
const TOP = 2;
/** Rows drawn beyond each edge of the view, so fast scrolling doesn't show gaps. */
const OVERSCAN = 8;
const COLUMNS = "grid-cols-[minmax(0,1fr)_72px_170px_80px_64px_72px] gap-4";

const HEADERS: { key: SortKey; label: string; align?: "right" }[] = [
  { key: "name", label: "Name" },
  { key: "size", label: "Size", align: "right" },
  { key: "progress", label: "Progress" },
  { key: "speed", label: "Speed", align: "right" },
  { key: "left", label: "Left", align: "right" },
  { key: "added", label: "Added", align: "right" },
];

type Props = {
  rows: Row[];
  selected: Set<number>;
  onSelect: (ids: Set<number>) => void;
  sort: Sort;
  onSort: (sort: Sort) => void;
  onOpen: (row: Row) => void;
  onDelete: () => void;
  onContextMenu: (row: Row, x: number, y: number) => void;
  /** A failed download's Retry. */
  onRetry: (row: Row) => void;
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
    const top = TOP + index * ROW_HEIGHT;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + ROW_HEIGHT > element.scrollTop + element.clientHeight)
      element.scrollTop = top + ROW_HEIGHT - element.clientHeight;
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
      role="grid"
      aria-label="Downloads"
      aria-rowcount={rows.length + 1}
      aria-multiselectable="true"
      className="flex min-h-0 flex-1 flex-col"
    >
      <div
        role="row"
        aria-rowindex={1}
        className={`grid ${COLUMNS} h-[30px] shrink-0 items-center px-6 text-[11.5px] font-semibold tracking-[0.06em] text-[#6b7390] uppercase`}
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
                className={`flex h-[30px] w-full items-center gap-1 whitespace-nowrap uppercase hover:text-[#c9cfdd] ${
                  align === "right" ? "justify-end" : ""
                }`}
              >
                {label}
                {sorted && <Icon name={sorted.descending ? "arrow-down" : "arrow-up"} className="size-3" />}
              </button>
            </div>
          );
        })}
      </div>
      <div
        ref={scroller}
        tabIndex={0}
        onKeyDown={keyDown}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        className="relative min-h-0 flex-1 overflow-y-auto outline-none focus-visible:ring-2 focus-visible:ring-[#6b88ff]/40 focus-visible:ring-inset"
      >
        {rows.length === 0 ? (
          <p className="px-6 py-16 text-center text-[13px] text-[#8b93a9]">Nothing here.</p>
        ) : (
          <div style={{ height: rows.length * ROW_HEIGHT + TOP * 2 }} className="relative">
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
                  onRetry={props.onRetry}
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

const DASH = "—";

/** A status as a rounded label in the Progress column. */
function StatusChip({
  tone,
  icon,
  children,
  title,
  onClick,
}: {
  tone: "green" | "red" | "plain" | "blue";
  icon?: IconName;
  children: ReactNode;
  title?: string;
  onClick?: () => void;
}) {
  const look = {
    green: { box: "border-[#12b88640] bg-[#12b8861a] text-[#7fe6c4]", dot: "bg-[#2fd3a0]" },
    red: { box: "border-[#e5484d4d] bg-[#e5484d1f] text-[#ff9a9e] hover:bg-[#e5484d33]", dot: "bg-[#f2555a]" },
    plain: { box: "border-white/10 bg-white/[0.05] text-[#c9cfdd]", dot: "bg-[#9aa3b8]" },
    blue: { box: "border-[#6b88ff4d] bg-[#4a6cff1f] text-[#a9bbff]", dot: "bg-[#6b88ff]" },
  }[tone];
  const className = `flex w-fit max-w-full items-center gap-1.5 rounded-full border px-[9px] py-[3px] text-xs font-medium whitespace-nowrap ${look.box}`;
  const inside = (
    <>
      {icon ? <Icon name={icon} className="size-3" /> : <span className={`size-1.5 shrink-0 rounded-full ${look.dot}`} />}
      <span className="truncate">{children}</span>
    </>
  );
  return onClick ? (
    <button
      type="button"
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className={className}
    >
      {inside}
    </button>
  ) : (
    <span title={title} className={className}>
      {inside}
    </span>
  );
}

/** "Night queue" → "Night". */
const queueShort = (name: string) => name.replace(/\s+queue$/i, "") || name;

function Progress({ row, onRetry }: { row: Row; onRetry: (row: Row) => void }) {
  switch (row.status) {
    case "completed":
      return <StatusChip tone="green">Completed</StatusChip>;
    case "failed":
      return (
        <StatusChip tone="red" icon="retry" title={row.error} onClick={() => onRetry(row)}>
          Failed · Retry
        </StatusChip>
      );
    case "queued":
      return (
        <StatusChip tone="plain" icon="clock">
          {row.queue ? `Queued · ${queueShort(row.queue)}` : "Waiting"}
        </StatusChip>
      );
    case "asking":
      return <StatusChip tone="blue">Asking you</StatusChip>;
  }
  const fraction = row.total ? Math.min(1, row.downloaded / row.total) : 0;
  const running = row.status === "running";
  const text = running
    ? row.total
      ? `${Math.floor(fraction * 100)}%`
      : formatBytes(row.downloaded)
    : (STATUS_LABEL[row.status] ?? row.status);
  return (
    <div className="flex items-center gap-2.5">
      <div className="h-1.5 w-[110px] shrink-0 rounded-full bg-white/[0.08]">
        <div
          className={`h-full rounded-full ${running ? "bar-fill" : "bg-[#6b7390]"}`}
          style={{ width: `${fraction * 100}%` }}
        />
      </div>
      <span
        className={`flex-1 truncate text-right text-xs tabular-nums ${
          running ? "font-semibold text-[#f4f6fb]" : "font-medium text-[#8b93a9]"
        }`}
      >
        {text}
      </span>
    </div>
  );
}

type RowProps = {
  row: Row;
  index: number;
  selected: boolean;
  cursor: boolean;
  onClick: (event: MouseEvent, index: number) => void;
  onDoubleClick: (row: Row) => void;
  onRetry: (row: Row) => void;
  onContextMenu: (event: MouseEvent) => void;
};

const DownloadRow = memo(function DownloadRow({
  row,
  index,
  selected,
  cursor,
  onClick,
  onDoubleClick,
  onRetry,
  onContextMenu,
}: RowProps) {
  const left = secondsLeft(row);
  const running = row.status === "running";
  const name = displayName(row);
  const value = (text: string | null) =>
    text ? <span className="font-medium text-[#f4f6fb]">{text}</span> : <span className="text-[#6b7390]">{DASH}</span>;
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
      style={{ transform: `translateY(${TOP + index * ROW_HEIGHT}px)` }}
      className={`absolute inset-x-3 top-0 grid ${COLUMNS} h-12 cursor-default items-center rounded-[10px] border px-3 text-[13px] select-none ${
        selected
          ? "border-[#6b88ff4d] bg-[#4a6cff1f]"
          : cursor
            ? "border-white/10"
            : "border-transparent hover:bg-white/[0.03]"
      }`}
    >
      <div role="gridcell" className="flex min-w-0 items-center gap-3" title={`${name}\n${row.url}`}>
        <FileIcon name={name} flat className="size-[18px]" />
        <span className={`truncate text-[#f4f6fb] ${selected ? "font-semibold" : "font-medium"}`}>{name}</span>
      </div>
      <div role="gridcell" className="text-right font-mono text-[#8b93a9] tabular-nums">
        {row.total != null ? formatBytes(row.total) : ""}
      </div>
      <div role="gridcell" className="min-w-0">
        <Progress row={row} onRetry={onRetry} />
        {row.error && !isActive(row.status) && <span className="sr-only">: {row.error}</span>}
      </div>
      <div role="gridcell" className="text-right font-mono tabular-nums">
        {value(running && row.speed != null ? formatSpeed(row.speed) : null)}
      </div>
      <div role="gridcell" className="text-right font-mono tabular-nums">
        {value(Number.isFinite(left) ? formatDuration(left) : null)}
      </div>
      <div
        role="gridcell"
        title={formatFullDate(row.addedAt)}
        className="text-right font-mono whitespace-nowrap text-[#8b93a9] tabular-nums"
      >
        {formatAdded(row.addedAt)}
      </div>
    </div>
  );
});
