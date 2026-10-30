// SPDX-License-Identifier: GPL-3.0-or-later

// What the downloads panel shows before the first download.

import picture from "../assets/empty.svg";
import { Icon } from "./Icon";

export function EmptyState({ onPaste, onExtension }: { onPaste: () => void; onExtension: () => void }) {
  return (
    <div data-view-in className="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto px-10 pb-2">
      <img src={picture} alt="" draggable={false} className="size-40" />
      <h2 className="mt-3.5 text-[22px] font-semibold tracking-[-0.02em] text-[#f4f6fb]">No downloads yet</h2>
      <p className="mt-2 text-center text-[13px] leading-normal text-[#8b93a9]">
        Click a download link in your browser and OmniDM catches it.
        <br />
        Or paste a link to start right away.
      </p>
      <div className="mt-[22px] flex gap-2.5">
        <button
          type="button"
          onClick={onPaste}
          className="button-primary flex h-[38px] items-center gap-2 rounded-[10px] pr-2 pl-3.5 text-[13px] font-semibold"
        >
          <Icon name="paste" />
          Paste a link
          <kbd className="rounded-[5px] bg-white/[0.18] px-1.5 py-0.5 font-sans text-[11.5px] font-semibold">Ctrl V</kbd>
        </button>
        <button
          type="button"
          onClick={onExtension}
          className="button-glass flex h-[38px] items-center gap-2 rounded-[10px] px-3.5 text-[13px] font-semibold"
        >
          <Icon name="puzzle" className="text-[#c9cfdd]" />
          Get the browser extension
        </button>
      </div>
    </div>
  );
}
