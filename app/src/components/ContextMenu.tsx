// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon, type IconName } from "./Icon";

export type MenuItem =
  | { label: string; icon?: IconName; disabled?: boolean; danger?: boolean; onSelect: () => void }
  | "separator";

/** A right-click menu at (`x`, `y`), kept inside the window. */
export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const element = menu.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    setPlace({
      left: Math.min(x, window.innerWidth - width - 4),
      top: Math.min(y, window.innerHeight - height - 4),
    });
    element.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [x, y]);

  useEffect(() => {
    const close = (event: Event) => {
      if (event instanceof MouseEvent && menu.current?.contains(event.target as Node)) return;
      onClose();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === "ArrowDown" ? at + 1 : at - 1;
        buttons[(next + buttons.length) % buttons.length]?.focus();
      }
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", key);
    };
  }, [onClose]);

  return (
    <div
      ref={menu}
      role="menu"
      style={place}
      onContextMenu={(e) => e.preventDefault()}
      className="fixed z-50 min-w-48 rounded-xl border border-white/[0.12] bg-[#141a2e]/95 p-1 text-[13px] text-[#f4f6fb] shadow-[0_16px_40px_-8px_rgb(0_0_0/0.6)] backdrop-blur-xl"
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} role="separator" className="my-1 h-px bg-white/[0.08]" />
        ) : (
          <button
            key={i}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
            className={`flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left outline-none disabled:opacity-35 ${
              item.danger
                ? "text-[#ff9a9e] hover:bg-[#e5484d]/15 focus:bg-[#e5484d]/15"
                : "hover:bg-white/[0.07] focus:bg-white/[0.07]"
            }`}
          >
            {item.icon ? <Icon name={item.icon} className="size-4 text-[#8b93a9]" /> : <span className="size-4" />}
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}
