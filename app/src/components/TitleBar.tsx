// SPDX-License-Identifier: GPL-3.0-or-later

// The main window's title bar, drawn by the app: the logo, the search box (Ctrl+K),
// Settings and the window buttons. Dragging it moves the window.

import type { RefObject } from "react";
import { Logo, WindowButtons } from "./Chrome";
import { Icon } from "./Icon";

type Props = {
  search: string;
  onSearch: (text: string) => void;
  searchInput: RefObject<HTMLInputElement | null>;
  onSettings: () => void;
  /** Settings is showing: its button stays lit. */
  settingsOpen: boolean;
};

export function TitleBar({ search, onSearch, searchInput, onSettings, settingsOpen }: Props) {
  return (
    <header data-tauri-drag-region className="grid h-12 shrink-0 grid-cols-[1fr_auto_1fr] items-center pr-2.5 pl-[18px]">
      <div data-tauri-drag-region className="flex items-center gap-2.5">
        <Logo className="size-5" />
        <span data-tauri-drag-region className="font-logo text-sm font-bold tracking-[-0.01em] text-[#f4f6fb]">
          OmniDM
        </span>
      </div>
      <label className="flex h-8 w-[380px] items-center gap-2 rounded-[10px] border border-white/[0.09] bg-white/[0.04] pr-1.5 pl-3 focus-within:border-[#6b88ff80] focus-within:bg-white/[0.06]">
        <span className="sr-only">Search downloads</span>
        <Icon name="search" className="size-3.5 text-[#8b93a9]" />
        <input
          ref={searchInput}
          type="search"
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              onSearch("");
              e.currentTarget.blur();
            }
          }}
          placeholder="Search downloads, links, sites…"
          className="min-w-0 flex-1 bg-transparent text-[12.5px] text-[#f4f6fb] outline-none placeholder:text-[#6b7390] [&::-webkit-search-cancel-button]:hidden"
        />
        <kbd className="rounded-[5px] border border-white/10 bg-white/[0.05] px-1.5 py-0.5 font-sans text-[11.5px] font-semibold text-[#8b93a9]">
          Ctrl K
        </kbd>
      </label>
      <div data-tauri-drag-region className="flex items-center justify-end gap-2.5">
        <button
          type="button"
          onClick={onSettings}
          aria-label="Settings"
          aria-pressed={settingsOpen}
          title="Settings"
          className={`flex size-8 items-center justify-center rounded-lg border ${
            settingsOpen
              ? "border-white/[0.12] bg-white/[0.08] text-[#f4f6fb]"
              : "border-transparent text-[#c9cfdd] hover:bg-white/[0.07] hover:text-[#f4f6fb]"
          }`}
        >
          <Icon name="settings" />
        </button>
        <span className="h-[18px] w-px bg-white/10" />
        <WindowButtons maximize big />
      </div>
    </header>
  );
}
