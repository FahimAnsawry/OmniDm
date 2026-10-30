// SPDX-License-Identifier: GPL-3.0-or-later

import { type CSSProperties, type ReactNode, useLayoutEffect, useRef } from "react";
import { animate } from "motion";

const prefersLess = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * A list whose highlights glide from item to item. Its items are marked `data-glide`.
 * The hover highlight follows the mouse and fades away when it leaves the list; the
 * selected item (`data-glide-current`) wears a glass box, which slides to a newly
 * selected item with a small spring.
 */
export function Glide({
  children,
  className = "",
  style,
  selected,
  selectedLook = "select-pill",
}: {
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  /** The selected box's look (index.css). */
  selectedLook?: string;
  /** Names the selected item: the box slides when this changes. */
  selected?: string;
}) {
  const box = useRef<HTMLDivElement>(null);
  const pill = useRef<HTMLDivElement>(null);
  const mark = useRef<HTMLDivElement>(null);
  /** Where the selected box is, and for which item, once it has been placed. */
  const placed = useRef<{ x: number; y: number; width: number; height: number; selected?: string } | null>(null);

  /** An item's place in the list, counting what is scrolled away. */
  const placeOf = (item: HTMLElement) => {
    const list = box.current!;
    const at = item.getBoundingClientRect();
    const origin = list.getBoundingClientRect();
    return {
      x: at.left - origin.left + list.scrollLeft,
      y: at.top - origin.top + list.scrollTop,
      width: at.width,
      height: at.height,
    };
  };

  // After every render: items can move without the choice changing (a queue appears
  // above), and the box keeps to its item; a new choice is what makes it slide.
  useLayoutEffect(() => {
    const list = box.current;
    const chosen = mark.current;
    if (!list || !chosen) return;
    const item = list.querySelector<HTMLElement>('[data-glide][data-glide-current]');
    if (!item) {
      chosen.style.opacity = "0";
      placed.current = null;
      return;
    }
    const to = placeOf(item);
    const from = placed.current;
    placed.current = { ...to, selected };
    if (from && from.selected === selected) {
      const moved = from.x !== to.x || from.y !== to.y || from.width !== to.width || from.height !== to.height;
      if (moved) animate(chosen, to, { duration: 0 });
      return;
    }
    if (!from || prefersLess()) {
      // Shown once in place: it would flash in the corner for a frame otherwise.
      void animate(chosen, to, { duration: 0 }).then(() => {
        if (placed.current?.selected === selected) chosen.style.opacity = "1";
      });
      return;
    }
    chosen.style.opacity = "1";
    animate(chosen, to, { type: "spring", visualDuration: 0.36, bounce: 0.18 });
  });

  const over = (target: EventTarget) => {
    const light = pill.current;
    const item = target instanceof Element ? target.closest<HTMLElement>("[data-glide]") : null;
    if (!box.current || !light || !item || !box.current.contains(item)) return;
    const to = placeOf(item);
    // Coming in from outside, it appears in place; from another item, it slides over.
    const fresh = light.style.opacity !== "1";
    light.style.transition = fresh ? "opacity 0.15s" : "";
    light.style.translate = `${to.x}px ${to.y}px`;
    light.style.width = `${to.width}px`;
    light.style.height = `${to.height}px`;
    light.style.opacity = "1";
  };

  return (
    <div
      ref={box}
      onPointerOver={(event) => over(event.target)}
      onPointerLeave={() => {
        if (pill.current) pill.current.style.opacity = "0";
      }}
      style={style}
      className={`relative isolate ${className}`}
    >
      <div ref={pill} aria-hidden className="glide-pill" />
      <div ref={mark} aria-hidden className={selectedLook} />
      {children}
    </div>
  );
}
