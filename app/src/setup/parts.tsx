// SPDX-License-Identifier: GPL-3.0-or-later

// Pieces shared by the setup window (OmniDM-Setup.exe) and the uninstall window
// (OmniDm.exe --uninstall): a 640×420 card with the title bar, a heading, frosted
// panels with rows, and the footer's buttons.

import type { ReactNode } from "react";
import { SmallTitleBar } from "../components/Chrome";
import { Icon, type IconName } from "../components/Icon";

/** The window's card: title bar, body and footer. `glow` tints the bottom-right light. */
export function SetupFrame({
  status,
  glow = "blue",
  footer,
  children,
  onClose,
}: {
  status: string;
  glow?: "blue" | "green" | "red";
  footer: ReactNode;
  children: ReactNode;
  onClose?: () => void;
}) {
  const look = glow === "green" ? "glow-done" : glow === "red" ? "glow-danger" : "glow-window";
  return (
    <div className={`${look} flex h-full flex-col px-4 pt-3 pb-4 font-sans text-[#f4f6fb] select-none`}>
      <SmallTitleBar status={status} onClose={onClose} />
      <div className="mt-6 flex flex-1 flex-col gap-5 px-2">{children}</div>
      <div className="flex items-center gap-2 px-2">{footer}</div>
    </div>
  );
}

/** A big heading with one or two lines under it, and an optional badge or logo on its left. */
export function Heading({ title, lead, badge }: { title: string; lead: ReactNode; badge?: ReactNode }) {
  return (
    <div className="flex items-center gap-[18px]">
      {badge}
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <h1 className="text-[22px] leading-tight font-bold tracking-[-0.02em]">{title}</h1>
        <p className="text-[13px] leading-normal text-[#8b93a9]">{lead}</p>
      </div>
    </div>
  );
}

/** A round badge beside a heading: green for done, red for removing. */
export function Badge({ icon, tone }: { icon: IconName; tone: "green" | "red" }) {
  const look =
    tone === "green"
      ? "border-[#2fd3a066] bg-[#12b88626] text-[#7fe6c4] shadow-[0_0_28px_rgb(47_211_160/0.3)]"
      : "border-[#e5484d4d] bg-[#e5484d1f] text-[#ff9a9e]";
  return (
    <span className={`flex size-14 shrink-0 items-center justify-center rounded-full border ${look}`}>
      <Icon name={icon} strokeWidth={2.5} className="size-7" />
    </span>
  );
}

/** A frosted box of rows, with a line between them. */
export function Panel({ children }: { children: ReactNode }) {
  return (
    <div className="divide-y divide-white/[0.07] rounded-[14px] border border-white/[0.08] bg-white/[0.03]">
      {children}
    </div>
  );
}

/** A checkbox in the window's style: a blue box with a white tick when checked. */
export function CheckBox({
  checked,
  onChange,
  disabled = false,
  id,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  id?: string;
}) {
  return (
    <span className="relative flex size-[18px] shrink-0 items-center justify-center">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="peer size-[18px] cursor-pointer appearance-none rounded-[5px] border border-white/[0.22] bg-white/[0.03] checked:bg-gradient-to-b checked:from-[#4a6cff] checked:to-[#2a48e0] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#6b88ff]"
      />
      <Icon
        name="check"
        strokeWidth={3}
        className="pointer-events-none absolute size-3 text-white opacity-0 peer-checked:opacity-100"
      />
    </span>
  );
}

/** A checkbox row: the box, an icon, a label and a line under it. */
export function CheckRow({
  checked,
  onChange,
  label,
  hint,
  icon,
  disabled = false,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  hint?: string;
  icon?: IconName;
  disabled?: boolean;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-3 px-3.5 py-2.5 hover:bg-white/[0.02]">
      <CheckBox checked={checked} onChange={onChange} disabled={disabled} />
      {icon && <Icon name={icon} className="text-[#8b93a9]" />}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className={`text-[13px] font-medium ${checked ? "text-[#f4f6fb]" : "text-[#c9cfdd]"}`}>{label}</span>
        {hint && <span className="text-xs text-[#8b93a9]">{hint}</span>}
      </span>
    </label>
  );
}

export type StepState = "done" | "now" | "next";

/** One step of the install: a tick when done, a spinner while working, a ring before. */
export function StepRow({ label, state }: { label: string; state: StepState }) {
  return (
    <div className="flex items-center gap-3 px-3.5 py-2.5">
      {state === "done" ? (
        <span className="flex size-5 items-center justify-center rounded-full border border-[#2fd3a066] bg-[#12b88626] text-[#7fe6c4]">
          <Icon name="check" strokeWidth={3} className="size-3" />
        </span>
      ) : state === "now" ? (
        <span className="flex size-5 items-center justify-center rounded-full border border-[#7c9bff80] bg-[#4a6cff33] text-[#b9c8ff]">
          <Icon name="loader" className="size-3 animate-spin" />
        </span>
      ) : (
        <span className="size-5 rounded-full border-[1.5px] border-white/[0.15] bg-white/[0.02]" />
      )}
      <span
        className={`flex-1 text-[13px] ${
          state === "now" ? "font-semibold text-[#f4f6fb]" : state === "done" ? "text-[#8b93a9]" : "text-[#8b93a9]"
        }`}
      >
        {label}
      </span>
      <span
        className={`text-xs font-medium ${
          state === "done" ? "text-[#7fe6c4]" : state === "now" ? "text-[#8ea4ff]" : "text-[#6b7390]"
        }`}
      >
        {state === "done" ? "Done" : state === "now" ? "Working…" : "Next"}
      </span>
    </div>
  );
}

/** Install or uninstall progress: the app's flowing blue bar with a glowing white head. */
export function ProgressBar({ fraction, label }: { fraction: number; label: string }) {
  const percent = Math.round(fraction * 100);
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      aria-label={label}
      className="relative h-2 rounded-full bg-white/[0.08]"
    >
      <div
        className="bar-flow relative h-full overflow-hidden rounded-full transition-[width] duration-300"
        style={{ width: `${percent}%` }}
      >
        <div className="bar-trail" />
      </div>
      <span
        className="absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_0_12px_2px_rgb(124_155_255/0.8)] transition-[left] duration-300"
        style={{ left: `${percent}%` }}
      />
    </div>
  );
}

/** A folder path in the numbers font, cut at the start when it is too long. */
export function PathText({ path, className = "" }: { path: string; className?: string }) {
  return (
    <span title={path} className={`truncate font-mono text-[13px] [direction:rtl] text-left ${className}`}>
      {/* The mark keeps a leading "C:\" the right way round in right-to-left mode. */}
      &lrm;{path}&lrm;
    </span>
  );
}
