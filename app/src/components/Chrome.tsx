// SPDX-License-Identifier: GPL-3.0-or-later

// Pieces of the dark window look shared by the main window and a download's window:
// the logo, the window buttons of a title bar drawn by the app, file icons and chips.

import { type ReactNode, useId } from "react";
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

/** A download window's title bar: the logo, "OmniDm · <status>", and its buttons. */
export function SmallTitleBar({ status, onClose }: { status?: string; onClose?: () => void }) {
  return (
    <div data-tauri-drag-region className="-my-1.5 -mr-1.5 flex items-center gap-2 select-none">
      <Logo />
      <span data-tauri-drag-region className="text-xs font-semibold text-[#9aa3b8]">
        OmniDm
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

/** A document with its extension on a coloured tag. `flat`: the small one in the list. */
export function FileIcon({ name, className, flat = false }: { name: string; className: string; flat?: boolean }) {
  const kind = fileKind(name);
  const rawId = useId();
  const id = rawId.replace(/[^a-zA-Z0-9_-]/g, "");
  const small = kind.tag.length > 3;
  return (
    <svg viewBox="0 0 64 76" aria-hidden="true" className={`shrink-0 ${className}`}>
      <defs>
        <linearGradient id={`${id}d`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#2a3150" />
          <stop offset="1" stopColor="#161b2e" />
        </linearGradient>
        <linearGradient id={`${id}g`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={kind.colors[0]} />
          <stop offset="1" stopColor={kind.colors[1]} />
        </linearGradient>
      </defs>
      {flat ? (
        <>
          <path
            d="M10 1h30l23 23v42a9 9 0 0 1-9 9H10a9 9 0 0 1-9-9V10a9 9 0 0 1 9-9z"
            fill="#232a44"
            stroke="#ffffff38"
            strokeWidth="2"
          />
          <path d="M40 1v13a10 10 0 0 0 10 10h13" fill="#ffffff1f" stroke="#ffffff38" strokeWidth="2" />
          {/* Drawn at ~26px wide, so the tag takes the full width and big letters, without an outline. */}
          <rect x="0" y="37" width="64" height="32" rx="6" fill={`url(#${id}g)`} />
          <text
            x="32"
            y={small ? 59 : 61}
            fontSize={small ? 17 : 22}
            fontWeight="800"
            fill="#ffffff"
            textAnchor="middle"
            fontFamily="Inter Variable, Segoe UI, sans-serif"
          >
            {kind.tag}
          </text>
        </>
      ) : (
        <>
          <path
            d="M10 .5h30l23.5 23.5V66a9.5 9.5 0 0 1-9.5 9.5H10A9.5 9.5 0 0 1 .5 66V10A9.5 9.5 0 0 1 10 .5z"
            fill={`url(#${id}d)`}
            stroke="#ffffff2e"
          />
          <path d="M40 .5V14a10 10 0 0 0 10 10h13.5" fill="#ffffff14" stroke="#ffffff2e" />
          <rect x="6" y="42" width="44" height="22" rx="6" fill={`url(#${id}g)`} stroke="#ffffff26" strokeWidth="1" />
          <text
            x="28"
            y="57"
            fontSize={small ? 11.5 : 13.5}
            fontWeight="800"
            fill="#ffffff"
            textAnchor="middle"
            letterSpacing="0.03em"
            fontFamily="Inter Variable, Segoe UI, sans-serif"
            style={{
              paintOrder: "stroke",
              stroke: "rgb(0 0 0 / 0.35)",
              strokeWidth: "1.5px",
              strokeLinejoin: "round",
            }}
          >
            {kind.tag}
          </text>
        </>
      )}
    </svg>
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
