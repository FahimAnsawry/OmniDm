// SPDX-License-Identifier: GPL-3.0-or-later

// The top of the downloads panel: the list's name and size, the actions on the
// selected downloads, and New download.

import type { ReactNode } from "react";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";
import { Icon, type IconName } from "./Icon";

type Props = {
  title: string;
  subtitle: string;
  /** No downloads at all: no actions to show. */
  empty: boolean;
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
  onSortMenu: (x: number, y: number) => void;
  onAdd: () => void;
};

export function PanelHeader(props: Props) {
  const { queue } = props;
  return (
    <div className="flex items-center gap-3 pt-[18px] pr-4 pb-3.5 pl-6">
      <div data-view-in className="flex min-w-0 flex-1 flex-col gap-[3px]">
        <h1 className="truncate text-[22px] font-semibold tracking-[-0.02em] text-[#f4f6fb]">{props.title}</h1>
        <p className="truncate text-xs text-[#8b93a9]">{props.subtitle}</p>
      </div>
      {!props.empty && (
        <>
          <div
            role="toolbar"
            aria-label="Actions"
            className="flex items-center gap-0.5 rounded-[10px] border border-white/[0.08] bg-white/[0.03] p-[3px]"
          >
            {queue &&
              (queue.running ? (
                <Button icon="stop" label="Stop queue" onClick={props.onStopQueue} />
              ) : (
                <Button
                  icon="play"
                  label="Start queue"
                  disabled={queue.downloadIds.length === 0}
                  onClick={props.onStartQueue}
                />
              ))}
            <Button
              icon="play"
              label="Resume"
              disabled={!props.canResume}
              strong={props.canResume && !props.canPause}
              onClick={props.onResume}
            />
            <Button icon="pause" label="Pause" disabled={!props.canPause} strong={props.canPause} onClick={props.onPause} />
            <Button icon="trash" label="Delete" disabled={!props.canDelete} danger onClick={props.onDelete} />
          </div>
          <button
            type="button"
            aria-label="Sort"
            title="Sort"
            onClick={(e) => {
              const box = e.currentTarget.getBoundingClientRect();
              props.onSortMenu(box.right - 192, box.bottom + 6);
            }}
            className="button-glass flex size-9 items-center justify-center rounded-[10px] text-[#c9cfdd]!"
          >
            <Icon name="arrow-up-down" />
          </button>
        </>
      )}
      {/* A glass button; its name and shortcut show in a tooltip below it. */}
      <button
        type="button"
        onClick={props.onAdd}
        aria-label="New download"
        aria-keyshortcuts="Control+N"
        className="group relative flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-white/[0.12] bg-white/[0.05] text-[#8ea4ff] hover:border-[#6b88ff]/50 hover:bg-[#4a6cff]/[0.18] hover:text-white hover:shadow-[0_6px_18px_-6px_rgb(59_91_255/0.6)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#6b88ff] active:bg-[#4a6cff]/25"
      >
        <Icon name="plus" className="size-[18px] transition-[rotate] duration-300 ease-out motion-safe:group-hover:rotate-90" />
        <span
          role="tooltip"
          className="pointer-events-none absolute top-full right-0 z-10 mt-2 flex items-center gap-2 rounded-lg border border-white/10 bg-[#1a1f30] py-1.5 pr-1.5 pl-2.5 text-xs font-medium whitespace-nowrap text-[#eef1f8] opacity-0 shadow-lg transition-opacity group-hover:opacity-100 group-hover:delay-400 group-focus-visible:opacity-100"
        >
          New download
          <kbd className="rounded-[5px] bg-white/[0.07] px-1.5 py-0.5 font-sans font-semibold text-[#9aa3b8]">Ctrl N</kbd>
        </span>
      </button>
    </div>
  );
}

function Button({
  icon,
  label,
  disabled,
  strong,
  danger,
  onClick,
}: {
  icon: IconName;
  label: ReactNode;
  disabled?: boolean;
  /** What the selection most likely wants. */
  strong?: boolean;
  /** Removes something: red. */
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`flex h-[30px] items-center gap-1.5 rounded-[7px] px-2.5 text-[12.5px] disabled:pointer-events-none disabled:opacity-35 ${
        danger
          ? "font-medium text-[#ff8a8f] hover:bg-[#e5484d]/[0.16] hover:text-[#ffc9cb] hover:shadow-[inset_0_0_0_1px_rgb(229_72_77/0.35)]"
          : strong
            ? "bg-white/[0.07] font-semibold text-[#f4f6fb]"
            : "font-medium text-[#c9cfdd] hover:bg-white/[0.07] hover:text-[#f4f6fb]"
      }`}
    >
      <Icon name={icon} className="size-3.5" />
      {label}
    </button>
  );
}
