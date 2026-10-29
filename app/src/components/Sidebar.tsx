// SPDX-License-Identifier: GPL-3.0-or-later

import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import type { ReactNode } from "react";
import { type Row, type View, inView, sameView } from "../lib/downloads";
import { Icon, type IconName } from "./Icon";

type Props = {
  rows: Row[];
  queues: QueueSummary[];
  categories: CategorySummary[];
  view: View;
  onView: (view: View) => void;
};

const STATUSES: { status: Extract<View, { kind: "status" }>["status"]; label: string; icon: IconName }[] = [
  { status: "all", label: "All downloads", icon: "all" },
  { status: "active", label: "Downloading", icon: "downloading" },
  { status: "paused", label: "Paused", icon: "paused" },
  { status: "completed", label: "Completed", icon: "completed" },
  { status: "failed", label: "Failed", icon: "failed" },
];

export function Sidebar({ rows, queues, categories, view, onView }: Props) {
  const count = (v: View) => rows.reduce((n, row) => n + (inView(row, v) ? 1 : 0), 0);

  const item = (v: View, label: string, icon: IconName, extra?: ReactNode) => {
    const selected = sameView(v, view);
    const n = count(v);
    return (
      <li key={`${v.kind}:${label}`}>
        <button
          type="button"
          aria-current={selected ? "page" : undefined}
          onClick={() => onView(v)}
          className={`flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] ${
            selected
              ? "bg-sky-600/10 font-medium text-sky-800 dark:bg-sky-400/15 dark:text-sky-200"
              : "text-neutral-700 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800"
          }`}
        >
          <Icon name={icon} className="size-4 opacity-70" />
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {extra}
          {n > 0 && <span className="text-xs tabular-nums text-neutral-500">{n}</span>}
        </button>
      </li>
    );
  };

  return (
    <nav
      aria-label="Download lists"
      className="flex w-52 shrink-0 flex-col gap-5 overflow-y-auto border-r border-neutral-200 bg-neutral-50 px-2 py-3 dark:border-neutral-800 dark:bg-neutral-900/60"
    >
      <ul className="space-y-0.5">
        {STATUSES.map(({ status, label, icon }) => item({ kind: "status", status }, label, icon))}
      </ul>
      {queues.length > 0 && (
        <Section title="Queues">
          {queues.map((q) =>
            item(
              { kind: "queue", name: q.name },
              q.name,
              "queue",
              q.running && (
                <span
                  title="Running"
                  className="size-1.5 rounded-full bg-emerald-500"
                  aria-label="running"
                />
              ),
            ),
          )}
        </Section>
      )}
      {categories.length > 0 && (
        <Section title="Categories">
          {categories.map((c) => item({ kind: "category", name: c.name }, c.name, "tag"))}
        </Section>
      )}
    </nav>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <h2 className="px-2.5 pb-1 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">
        {title}
      </h2>
      <ul className="space-y-0.5">{children}</ul>
    </div>
  );
}
