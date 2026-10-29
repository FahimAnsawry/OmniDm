// SPDX-License-Identifier: GPL-3.0-or-later

import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import type { ReactNode } from "react";
import { type Row, type View, inView, sameView } from "../lib/downloads";
import { formatSpeed } from "../lib/format";
import { Icon, type IconName } from "./Icon";
import { SpeedGraph } from "./SpeedGraph";

type Props = {
  rows: Row[];
  queues: QueueSummary[];
  categories: CategorySummary[];
  view: View;
  onView: (view: View) => void;
  onAdd: () => void;
  /** All running downloads together, and the last minute of it. */
  speed: number;
  speeds: number[];
};

const STATUSES: { status: Extract<View, { kind: "status" }>["status"]; label: string; icon: IconName }[] = [
  { status: "all", label: "All downloads", icon: "layers" },
  { status: "active", label: "Downloading", icon: "downloading" },
  { status: "paused", label: "Paused", icon: "paused" },
  { status: "completed", label: "Completed", icon: "completed" },
  { status: "failed", label: "Failed", icon: "failed" },
];

/** An icon for a category, by its usual names. */
function categoryIcon(name: string): IconName {
  const n = name.toLowerCase();
  if (/program|software|app/.test(n)) return "app-window";
  if (/video|movie|film/.test(n)) return "clapperboard";
  if (/music|audio|sound/.test(n)) return "music";
  if (/doc|book|pdf/.test(n)) return "file-text";
  if (/image|picture|photo/.test(n)) return "image";
  return "tag";
}

export function Sidebar({ rows, queues, categories, view, onView, onAdd, speed, speeds }: Props) {
  const count = (v: View) => rows.reduce((n, row) => n + (inView(row, v) ? 1 : 0), 0);
  const running = rows.filter((r) => r.status === "running").length;

  const item = (v: View, label: string, icon: IconName, extra?: ReactNode) => {
    const selected = sameView(v, view);
    return (
      <li key={`${v.kind}:${label}`}>
        <button
          type="button"
          aria-current={selected ? "page" : undefined}
          onClick={() => onView(v)}
          className={`flex h-[34px] w-full items-center gap-2.5 rounded-[9px] border px-2.5 text-left text-[13px] ${
            selected
              ? "border-white/10 bg-white/[0.06] font-semibold text-[#f4f6fb]"
              : "border-transparent font-medium text-[#c9cfdd] hover:bg-white/[0.04]"
          }`}
        >
          <Icon name={icon} className={selected ? "text-[#8ea4ff]" : "text-[#8b93a9]"} />
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {extra}
        </button>
      </li>
    );
  };

  const number = (n: number) => <span className="text-xs font-medium text-[#8b93a9] tabular-nums">{n}</span>;

  return (
    <nav aria-label="Download lists" className="flex w-[232px] shrink-0 flex-col gap-3.5 px-3 pt-1 pb-3">
      <button
        type="button"
        onClick={onAdd}
        title="New download (Ctrl+N)"
        className="button-primary flex h-[38px] shrink-0 items-center gap-2 rounded-[10px] pr-2 pl-3 text-[13px] font-semibold"
      >
        <Icon name="plus" />
        <span className="flex-1 text-left">New download</span>
        <kbd className="rounded-[5px] bg-white/[0.18] px-1.5 py-0.5 font-sans text-[11.5px] font-semibold">Ctrl N</kbd>
      </button>

      <div className="-mr-2 flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto pr-2 [&:not(:hover)::-webkit-scrollbar-thumb]:bg-transparent">
        <ul className="space-y-0.5">
          {STATUSES.map(({ status, label, icon }) => {
            const v: View = { kind: "status", status };
            const n = count(v);
            return item(
              v,
              label,
              icon,
              status === "failed" && n > 0 ? (
                <span className="rounded-full bg-[#e5484d] px-1.5 text-[11.5px] font-bold text-white tabular-nums">
                  {n}
                </span>
              ) : (
                number(n)
              ),
            );
          })}
        </ul>
        {queues.length > 0 && (
          <Section title="Queues">
            {queues.map((q) =>
              item(
                { kind: "queue", name: q.name },
                q.name,
                q.schedule ? "moon" : "queue",
                q.running ? (
                  <span
                    title="Running"
                    aria-label="running"
                    className="mr-1 size-1.5 rounded-full bg-[#2fd3a0] shadow-[0_0_6px_#2fd3a0]"
                  />
                ) : q.nextStart ? (
                  <span title={`Starts ${q.nextStart}`} className="text-xs font-medium text-[#8b93a9] tabular-nums">
                    {q.nextStart.slice(-5)}
                  </span>
                ) : undefined,
              ),
            )}
          </Section>
        )}
        {categories.length > 0 && (
          <Section title="Categories">
            {categories.map((c) => {
              const v: View = { kind: "category", name: c.name };
              const n = count(v);
              return item(v, c.name, categoryIcon(c.name), n > 0 ? number(n) : undefined);
            })}
          </Section>
        )}
      </div>

      <div role="status" className="flex shrink-0 flex-col gap-1.5 rounded-xl border border-white/[0.08] bg-white/[0.03] p-3">
        <div className="flex items-center gap-1.5 text-xs font-medium text-[#8b93a9]">
          <span
            className={`size-1.5 rounded-full ${
              running ? "bg-[#7c9bff] shadow-[0_0_6px_#7c9bff]" : "bg-[#6b7390]"
            }`}
          />
          {running ? `${running} downloading` : "Idle"}
          <Icon name="arrow-down" className={`ml-auto size-3 ${running ? "text-[#8ea4ff]" : "text-[#6b7390]"}`} />
        </div>
        <p
          className={`text-lg font-semibold tracking-[-0.01em] tabular-nums ${
            running ? "text-[#f4f6fb]" : "text-[#c9cfdd]"
          }`}
        >
          {formatSpeed(speed)}
        </p>
        <SpeedGraph samples={running ? speeds : []} height={28} className="h-7 w-full" />
      </div>
    </nav>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <h2 className="px-2.5 pb-1 text-[11.5px] font-semibold tracking-[0.08em] text-[#6b7390] uppercase">{title}</h2>
      <ul className="space-y-0.5">{children}</ul>
    </div>
  );
}
