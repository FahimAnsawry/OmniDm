// SPDX-License-Identifier: GPL-3.0-or-later

// Pieces of the dark window look shared by the main window and a download's window:
// the logo, the window buttons of a title bar drawn by the app, file icons and chips.

import type { ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import logo from "../assets/logo.svg";
import { fileKind } from "../lib/fileKind";
import { Icon, type IconName } from "./Icon";
import { closeWindow, minimizeWindow } from "./WindowMotion";

export function Logo({ className = "size-4" }: { className?: string }) {
  return <img src={logo} alt="" draggable={false} className={`shrink-0 ${className}`} />;
}

/** Minimize, maximize (in a resizable window) and close, for a window without the system's title bar. */
export function WindowButtons({
  maximize = false,
  big = false,
  onClose,
}: {
  maximize?: boolean;
  big?: boolean;
  onClose?: () => void;
}) {
  const window = getCurrentWindow();
  const button = (label: string, icon: IconName, onClick: () => void, danger = false) => (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`flex items-center justify-center rounded-lg text-[#8b93a9] ${
        big ? "size-8" : "size-7"
      } ${danger ? "hover:bg-[#e5484d] hover:text-white" : "hover:bg-white/[0.07] hover:text-[#f4f6fb]"}`}
    >
      <Icon name={icon} className={icon === "square" ? "size-3" : "size-4"} />
    </button>
  );
  return (
    <div className={`flex items-center ${big ? "gap-0" : "gap-1"}`}>
      {button("Minimize", "minus", () => void minimizeWindow())}
      {maximize && button("Maximize", "square", () => void window.toggleMaximize())}
      {button("Close", "close", onClose ?? (() => void closeWindow()), true)}
    </div>
  );
}

/** A download window's title bar: the logo, "OmniDM · <status>", and its buttons. */
export function SmallTitleBar({ status, onClose }: { status?: string; onClose?: () => void }) {
  return (
    <div data-tauri-drag-region className="-my-1.5 -mr-1.5 flex items-center gap-2 select-none">
      <Logo />
      <span data-tauri-drag-region className="font-logo text-xs font-semibold text-[#9aa3b8]">
        OmniDM
      </span>
      {status && (
        <span data-tauri-drag-region className="truncate text-xs text-[#6b7390]">
          ·&nbsp; {status}
        </span>
      )}
      <span data-tauri-drag-region className="h-7 flex-1" />
      <WindowButtons onClose={onClose} />
    </div>
  );
}

/**
 * The file type's line icon. `flat`: bare and in its colour, for the main window's list and
 * Add download; otherwise plain, centered in a tile, for the download windows.
 */
export function FileIcon({ name, className, flat = false }: { name: string; className: string; flat?: boolean }) {
  const { icon, color } = fileKind(name);
  if (flat) {
    return (
      <span aria-hidden="true" style={{ color }} className={`flex shrink-0 ${className}`}>
        <Icon name={icon} strokeWidth={1.75} className="size-full" />
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      className={`flex shrink-0 items-center justify-center rounded-[14px] border border-white/[0.1] bg-white/[0.05] text-[#c9cfdd] ${className}`}
    >
      <Icon name={icon} strokeWidth={1.5} className="size-[46%]" />
    </span>
  );
}

/** A small label in a rounded box. */
export function Chip({ children, tone = "plain" }: { children: ReactNode; tone?: "plain" | "green" }) {
  return (
    <span
      className={`flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap tabular-nums ${
        tone === "green"
          ? "border-[#12b88640] bg-[#12b8861a] text-[#7fe6c4]"
          : "border-white/[0.09] bg-white/[0.05] text-[#d5dae6]"
      }`}
    >
      {tone === "green" && <span className="size-1.5 rounded-full bg-[#2fd3a0]" />}
      {children}
    </span>
  );
}

/** The ⏎ on a main button: Enter does the same. */
export function EnterHint() {
  return (
    <span className="flex size-5 items-center justify-center rounded-[5px] bg-white/[0.18]">
      <Icon name="enter" className="size-3" />
    </span>
  );
}

/** A folder as "Downloads › Programs": from the downloads folder when it's inside it. */
export function folderCrumbs(folder: string, downloadsFolder?: string): string[] {
  const split = (path: string) => path.split(/[\\/]+/).filter(Boolean);
  const parts = split(folder);
  if (downloadsFolder) {
    const base = split(downloadsFolder);
    const inside =
      base.length > 0 &&
      base.length <= parts.length &&
      base.every((part, i) => part.toLowerCase() === parts[i].toLowerCase());
    if (inside) return [base[base.length - 1], ...parts.slice(base.length)].slice(-3);
  }
  return parts.slice(-2);
}

export function Crumbs({ crumbs, size = "text-[13px]" }: { crumbs: string[]; size?: string }) {
  return (
    <span className={`flex min-w-0 items-center gap-1 ${size}`}>
      {crumbs.map((crumb, index) => {
        const last = index === crumbs.length - 1;
        return (
          <span key={index} className={`flex items-center gap-1 ${last ? "min-w-0" : "shrink-0"}`}>
            {index > 0 && <Icon name="right" className="size-3 text-[#6b7390]" />}
            <span className={`truncate ${last ? "font-semibold text-[#f4f6fb]" : "text-[#c9cfdd]"}`}>{crumb}</span>
          </span>
        );
      })}
    </span>
  );
}
