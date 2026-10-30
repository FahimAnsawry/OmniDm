// SPDX-License-Identifier: GPL-3.0-or-later

// One download's details, in its own window: the map of its pieces, its connections,
// what the server said, and its log.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { DownloadDetails } from "../../../protocol/ts/DownloadDetails";
import type { SegmentInfo } from "../../../protocol/ts/SegmentInfo";
import { api, errorText } from "../lib/api";
import { STATUS_LABEL, canResume, displayName, isActive } from "../lib/downloads";
import { formatBytes, formatDuration, formatSpeed } from "../lib/format";
import { Button } from "./Fields";
import { Icon } from "./Icon";

/** How often the window asks for fresh details. */
const REFRESH_MS = 500;

type Tab = "connections" | "server" | "log";

export function DetailsWindow({ id }: { id: number }) {
  const [details, setDetails] = useState<DownloadDetails | null>(null);
  const [gone, setGone] = useState(false);
  const [tab, setTab] = useState<Tab>("connections");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stopped = false;
    const load = () =>
      api
        .details(id)
        .then((found) => {
          if (stopped) return;
          if (found) setDetails(found);
          else setGone(true);
        })
        .catch((e) => setError(errorText(e)));
    load();
    const timer = window.setInterval(load, REFRESH_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [id]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") getCurrentWindow().close();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  const name = details ? displayName(details.summary) : "";
  useEffect(() => {
    if (name) getCurrentWindow().setTitle(`Details · ${name}`).catch(() => {});
  }, [name]);

  if (gone) {
    return (
      <Shell>
        <p className="text-sm text-neutral-500">This download is no longer in the list.</p>
      </Shell>
    );
  }
  if (!details) {
    return (
      <Shell>
        <p className="text-sm text-neutral-500">{error ?? "Loading…"}</p>
      </Shell>
    );
  }

  const { summary } = details;
  const total = summary.total ?? (details.segments.length ? details.segments[details.segments.length - 1].end + 1 : undefined);
  const done = summary.status === "completed";
  const downloaded = done && total != null ? total : summary.downloaded;
  const percent = total ? Math.floor((downloaded / total) * 100) : done ? 100 : 0;
  const left = details.speed && total != null ? (total - downloaded) / details.speed : NaN;
  const act = (action: Promise<unknown>) => action.then(() => setError(null)).catch((e) => setError(errorText(e)));

  return (
    <Shell>
      <header className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-[15px] font-semibold" title={name}>
            {name}
          </h1>
          <p className="truncate text-xs text-neutral-500 select-text" title={summary.url}>
            {summary.url}
          </p>
        </div>
        <div className="text-right">
          <p className="font-mono text-2xl font-semibold tabular-nums">{percent}%</p>
          <p className="text-xs text-neutral-500">{STATUS_LABEL[summary.status] ?? summary.status}</p>
        </div>
      </header>

      <dl className="grid grid-cols-4 gap-3 text-[13px] tabular-nums">
        <Stat label="Downloaded">
          {formatBytes(downloaded)}
          {total != null && <span className="text-neutral-500"> of {formatBytes(total)}</span>}
        </Stat>
        <Stat label="Speed">{details.speed != null ? formatSpeed(details.speed) : "—"}</Stat>
        <Stat label="Time left">{Number.isFinite(left) ? formatDuration(left) : "—"}</Stat>
        <Stat label="Connections">{details.speed != null ? details.connections : "—"}</Stat>
      </dl>

      <SegmentMap segments={details.segments} total={total} done={done} />

      <div role="tablist" aria-label="Details" className="flex gap-1 border-b border-neutral-200 dark:border-neutral-800">
        {(
          [
            ["connections", "Pieces & connections"],
            ["server", "Server"],
            ["log", "Log"],
          ] as [Tab, string][]
        ).map(([value, label]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={tab === value}
            onClick={() => setTab(value)}
            className={`-mb-px border-b-2 px-3 py-1.5 text-[13px] font-medium ${
              tab === value
                ? "border-sky-600 text-sky-800 dark:border-sky-400 dark:text-sky-200"
                : "border-transparent text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <div role="tabpanel" className="min-h-0 flex-1 overflow-auto">
        {tab === "connections" ? (
          <Pieces segments={details.segments} resumable={details.resumable} />
        ) : tab === "server" ? (
          <ServerInfo details={details} />
        ) : (
          <Log lines={details.log} />
        )}
      </div>

      {error && (
        <p role="alert" className="text-[13px] text-red-700 dark:text-red-400">
          {error}
        </p>
      )}
      {summary.error && !isActive(summary.status) && (
        <p className="text-[13px] text-red-700 dark:text-red-400">{summary.error}</p>
      )}

      <footer className="flex items-center gap-2">
        <Button onClick={() => act(api.showInFolder(id))} className="mr-auto">
          <Icon name="folder" className="size-4 opacity-70" />
          Show in folder
        </Button>
        {isActive(summary.status) && (
          <Button onClick={() => act(api.pause([id]))}>
            <Icon name="pause" className="size-4" />
            Pause
          </Button>
        )}
        {canResume(summary.status) && (
          <Button onClick={() => act(api.resume([id]))}>
            <Icon name="play" className="size-4" />
            Resume
          </Button>
        )}
        {done && (
          <Button onClick={() => act(api.openFile(id))}>
            <Icon name="open" className="size-4" />
            Open
          </Button>
        )}
        <Button primary onClick={() => getCurrentWindow().close()}>
          Close
        </Button>
      </footer>
    </Shell>
  );
}

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-screen flex-col gap-4 bg-white p-5 font-sans text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      {children}
    </div>
  );
}

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-xs text-neutral-500">{label}</dt>
      <dd className="mt-0.5 font-mono font-medium">{children}</dd>
    </div>
  );
}

const pieceColor = (s: SegmentInfo) =>
  s.written > s.end - s.start ? "bg-emerald-500" : s.active ? "bg-sky-500" : "bg-neutral-400 dark:bg-neutral-500";

/** The whole file as one bar: each piece in its place, filled as far as it has got. */
function SegmentMap({ segments, total, done }: { segments: SegmentInfo[]; total?: number; done: boolean }) {
  if (!total || segments.length === 0) {
    return (
      <div className="space-y-1">
        <div className="h-5 overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800">
          {done && <div className="h-full bg-emerald-500" />}
        </div>
        <p className="text-xs text-neutral-500">
          {done ? "Complete." : "The map of pieces shows once the server has said how big the file is."}
        </p>
      </div>
    );
  }
  return (
    <div className="space-y-1.5">
      <div
        role="img"
        aria-label={`${segments.length} pieces, ${segments.filter((s) => s.active).length} downloading now`}
        className="relative h-5 overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800"
      >
        {segments.map((s) => {
          const length = s.end - s.start + 1;
          return (
            <div
              key={s.start}
              className="absolute inset-y-0 border-r border-white/70 dark:border-neutral-900/70"
              style={{ left: `${(s.start / total) * 100}%`, width: `${(length / total) * 100}%` }}
            >
              <div className={`h-full ${pieceColor(s)}`} style={{ width: `${Math.min(1, s.written / length) * 100}%` }} />
            </div>
          );
        })}
      </div>
      <p className="flex gap-4 text-xs text-neutral-500">
        <Legend className="bg-emerald-500">Done</Legend>
        <Legend className="bg-sky-500">Downloading</Legend>
        <Legend className="bg-neutral-400 dark:bg-neutral-500">Waiting</Legend>
        <span className="ml-auto">{segments.length} {segments.length === 1 ? "piece" : "pieces"}</span>
      </p>
    </div>
  );
}

function Legend({ className, children }: { className: string; children: ReactNode }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className={`size-2.5 rounded-sm ${className}`} />
      {children}
    </span>
  );
}

function Pieces({ segments, resumable }: { segments: SegmentInfo[]; resumable: boolean }) {
  if (segments.length === 0) {
    return (
      <p className="py-3 text-[13px] text-neutral-500">
        {resumable
          ? "No pieces yet."
          : "The server can't send parts of this file, so it downloads on one connection, start to end."}
      </p>
    );
  }
  return (
    <table className="w-full text-[13px] tabular-nums">
      <thead className="text-left text-xs text-neutral-500">
        <tr>
          <th className="py-1.5 pr-3 font-medium">#</th>
          <th className="py-1.5 pr-3 font-medium">Bytes</th>
          <th className="py-1.5 pr-3 text-right font-medium">Done</th>
          <th className="py-1.5 pr-3 text-right font-medium">Speed</th>
          <th className="py-1.5 font-medium">State</th>
        </tr>
      </thead>
      <tbody>
        {segments.map((s, i) => {
          const length = s.end - s.start + 1;
          const complete = s.written >= length;
          return (
            <tr key={s.start} className="border-t border-neutral-100 dark:border-neutral-800">
              <td className="py-1.5 pr-3 font-mono text-neutral-500">{i + 1}</td>
              <td className="py-1.5 pr-3 font-mono">
                {formatBytes(s.start)} – {formatBytes(s.end + 1)}
              </td>
              <td className="py-1.5 pr-3 text-right font-mono">
                {formatBytes(s.written)} <span className="text-neutral-500">/ {formatBytes(length)}</span>
              </td>
              <td className="py-1.5 pr-3 text-right font-mono">{s.active ? formatSpeed(s.speed) : ""}</td>
              <td className={`py-1.5 ${complete ? "text-emerald-700 dark:text-emerald-400" : s.active ? "text-sky-700 dark:text-sky-400" : "text-neutral-500"}`}>
                {complete ? "Done" : s.active ? "Downloading" : "Waiting"}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function ServerInfo({ details }: { details: DownloadDetails }) {
  const rows: [string, ReactNode][] = [
    ["Link", details.summary.url],
    ...details.redirects.map((hop, i): [string, ReactNode] => [
      i === 0 ? "Redirects" : "",
      <span key={i}>
        <span className="text-neutral-500">{hop.status} →</span> {hop.url}
      </span>,
    ]),
    ["File comes from", details.finalUrl ?? "Not asked yet"],
    ["Request", details.method],
    ["File type", details.contentType ?? "—"],
    ["Size", details.summary.total != null ? `${formatBytes(details.summary.total)} (${details.summary.total.toLocaleString()} bytes)` : "Unknown"],
    ["Resumable", details.resumable ? "Yes: it can be split into pieces and continued later" : "No"],
    ["ETag", details.etag ?? "—"],
    ["Last modified", details.lastModified ?? "—"],
    ["Referrer", details.referrer ?? "—"],
    ...details.requestHeaders.map((h, i): [string, ReactNode] => [
      i === 0 ? "Sent headers" : "",
      <span key={h.name}>
        <span className="text-neutral-500">{h.name}:</span> {h.value}
      </span>,
    ]),
    ["Saved in", details.summary.saveDir],
  ];
  return (
    <table className="w-full text-[13px]">
      <tbody>
        {rows.map(([label, value], i) => (
          <tr key={i} className={label ? "border-t border-neutral-100 dark:border-neutral-800" : ""}>
            <th scope="row" className="w-36 py-1.5 pr-3 text-left align-top text-xs font-medium text-neutral-500">
              {label}
            </th>
            <td className="py-1.5 break-all select-text">{value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });

function Log({ lines }: { lines: DownloadDetails["log"] }) {
  const end = useRef<HTMLDivElement>(null);
  const count = lines.length;
  // Follow new lines as they come.
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [count]);
  if (count === 0) {
    return <p className="py-3 text-[13px] text-neutral-500">Nothing yet. The log starts when OmniDM starts.</p>;
  }
  return (
    <div className="py-2">
      <ol className="space-y-0.5 font-mono text-xs select-text">
        {lines.map((line, i) => (
          <li key={i} className="flex gap-3">
            <time className="shrink-0 text-neutral-500 tabular-nums">{clock.format(new Date(line.at))}</time>
            <span className="break-words">{line.text}</span>
          </li>
        ))}
      </ol>
      <div ref={end} />
    </div>
  );
}
