// SPDX-License-Identifier: GPL-3.0-or-later

// The top of the downloads panel: the list's name and size, and the actions on the
// selected downloads.

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
};

export function PanelHeader(props: Props) {
  const { queue } = props;
  return (
    <div className="flex items-center gap-3 pt-[18px] pr-4 pb-3.5 pl-6">
      <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
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
            <Button icon="trash" label="Delete" disabled={!props.canDelete} onClick={props.onDelete} />
          </div>
          <button
            type="button"
            aria-label="Sort"
            title="Sort"
            onClick={(e) => {
              const box = e.currentTarget.getBoundingClientRect();
              props.onSortMenu(box.right - 192, box.bottom + 6);
            }}
            className="flex size-9 items-center justify-center rounded-[10px] border border-white/[0.08] bg-white/[0.03] text-[#c9cfdd] hover:bg-white/[0.07]"
          >
            <Icon name="arrow-up-down" />
          </button>
        </>
      )}
    </div>
  );
}

function Button({
  icon,
  label,
  disabled,
  strong,
  onClick,
}: {
  icon: IconName;
  label: ReactNode;
  disabled?: boolean;
  /** What the selection most likely wants. */
  strong?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`flex h-[30px] items-center gap-1.5 rounded-[7px] px-2.5 text-[12.5px] disabled:pointer-events-none disabled:opacity-35 ${
        strong
          ? "bg-white/[0.07] font-semibold text-[#f4f6fb]"
          : "font-medium text-[#c9cfdd] hover:bg-white/[0.07] hover:text-[#f4f6fb]"
      }`}
    >
      <Icon name={icon} className="size-3.5" />
      {label}
    </button>
  );
}
