// SPDX-License-Identifier: GPL-3.0-or-later

import type { ReactNode } from "react";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { Icon, type IconName } from "./Icon";

type Props = {
  onAdd: () => void;
  onSettings: () => void;
  canResume: boolean;
  canPause: boolean;
  canDelete: boolean;
  onResume: () => void;
  onPause: () => void;
  onDelete: () => void;
  /** The queue being looked at, if any. */
  queue?: QueueSummary;
  onStartQueue: () => void;
  onStopQueue: () => void;
  search: string;
  onSearch: (text: string) => void;
};

export function Toolbar(props: Props) {
  const { queue } = props;
  return (
    <div
      role="toolbar"
      aria-label="Actions"
      className="flex items-center gap-1 border-b border-neutral-200 bg-white px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900"
    >
      <button
        type="button"
        onClick={props.onAdd}
        title="Add download (Ctrl+N)"
        className="mr-2 flex items-center gap-1.5 rounded-md bg-sky-600 px-3 py-1.5 text-[13px] font-medium text-white hover:bg-sky-700 dark:hover:bg-sky-500"
      >
        <Icon name="plus" className="size-4" />
        Add
      </button>
      <Button icon="play" label="Resume" disabled={!props.canResume} onClick={props.onResume} />
      <Button icon="pause" label="Pause" disabled={!props.canPause} onClick={props.onPause} />
      <Button icon="trash" label="Delete" disabled={!props.canDelete} onClick={props.onDelete} />
      {queue && (
        <>
          <span className="mx-2 h-5 w-px bg-neutral-200 dark:bg-neutral-700" />
          {queue.running ? (
            <Button icon="stop" label="Stop queue" onClick={props.onStopQueue} />
          ) : (
            <Button
              icon="play"
              label="Start queue"
              disabled={queue.downloadIds.length === 0}
              onClick={props.onStartQueue}
            />
          )}
          {queue.nextStart && !queue.running && (
            <span className="ml-1 text-xs text-neutral-500">Starts {queue.nextStart}</span>
          )}
        </>
      )}
      <div className="ml-auto flex items-center">
        <label className="relative">
          <span className="sr-only">Search downloads</span>
          <Icon
            name="search"
            className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-neutral-400"
          />
          <input
            type="search"
            value={props.search}
            onChange={(e) => props.onSearch(e.target.value)}
            placeholder="Search"
            className="w-56 rounded-md border border-neutral-300 bg-white py-1 pr-2 pl-7 text-[13px] placeholder:text-neutral-400 focus:border-sky-500 focus:ring-2 focus:ring-sky-500/30 focus:outline-none dark:border-neutral-700 dark:bg-neutral-950"
          />
        </label>
        <button
          type="button"
          onClick={props.onSettings}
          aria-label="Settings"
          title="Settings"
          className="ml-1 rounded-md p-1.5 text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
        >
          <Icon name="settings" className="size-[18px]" />
        </button>
      </div>
    </div>
  );
}

function Button({
  icon,
  label,
  disabled,
  onClick,
}: {
  icon: IconName;
  label: ReactNode;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[13px] font-medium text-neutral-700 hover:bg-neutral-100 disabled:pointer-events-none disabled:opacity-35 dark:text-neutral-200 dark:hover:bg-neutral-800"
    >
      <Icon name={icon} className="size-4" />
      {label}
    </button>
  );
}
