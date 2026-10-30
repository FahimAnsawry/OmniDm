// SPDX-License-Identifier: GPL-3.0-or-later

// Opens and closes a window with a premium motion. The window is see-through and a
// little larger than what it shows: a rounded card sits inside it, with a soft shadow
// drawn by the page (the system shadow puts a white line around a window like this).
//
// Opening: the card comes into focus out of a blur, rising and growing with a small
// spring; its parts follow one after another and a light sweeps across it once.
// Closing: it shrinks back into a blur, quickly. Minimizing: it sinks toward the taskbar,
// and rises back out of it when restored.
//
// The page decides when the card shows. The app tells it with a "window-reveal" event
// each time it shows the window; the window coming back from the taskbar or getting
// focus does too, so a window that Windows didn't let take focus still shows.

import { type ReactNode, useEffect, useRef } from "react";
import { animate, stagger } from "motion";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>["startResizeDragging"]>[0];

/** The see-through space around the card for its shadow, in px (`--frame-inset` in index.css). */
const INSET = 20;

type Pose = { opacity: number; scale: number; y: number; filter: string };

const shown: Pose = { opacity: 1, scale: 1, y: 0, filter: "blur(0px)" };
/** Before opening: small, low and out of focus. */
const born: Pose = { opacity: 0, scale: 0.9, y: 18, filter: "blur(14px)" };
const closed: Pose = { opacity: 0, scale: 0.95, y: 8, filter: "blur(8px)" };
/** Gone into the taskbar. */
const sunk: Pose = { opacity: 0, scale: 0.84, y: 64, filter: "blur(6px)" };

/** `born` as the card's style from its very first frame: no flash of it before it opens. */
const BORN_STYLE = { opacity: 0, transform: "translateY(18px) scale(0.9)", filter: "blur(14px)" };

/** Opens with a lively spring for a small window, a calm one for the main window. */
const BOUNCE = { popup: 0.26, main: 0.1 };

type Leave = "close" | "minimize";

/** The window's card, while a `WindowMotion` is on the page. */
let card: { enter: () => void; leave: (how: Leave) => Promise<boolean> } | null = null;

/** The space around the card, in px: none while the window is maximized. */
export function frameInset() {
  return document.documentElement.classList.contains("maximized") ? 0 : INSET;
}

/** Closes the card away, then closes the window. */
export async function closeWindow() {
  if ((await card?.leave("close")) === false) return;
  await getCurrentWindow().close();
}

/** Closes the card away, then closes the window without asking its page. */
export async function destroyWindow() {
  if ((await card?.leave("close")) === false) return;
  const win = getCurrentWindow();
  await win.destroy().catch(() => win.close());
}

/** Sinks the card toward the taskbar, then minimizes the window; it rises back when restored. */
export async function minimizeWindow() {
  if ((await card?.leave("minimize")) === false) return;
  await getCurrentWindow().minimize();
}

/** Brings the card back, when a close was called off. */
export function showWindowContent() {
  card?.enter();
}

const prefersLess = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The fonts, loaded (or given up on): opening before would redraw the text in them midway. */
const fontsReady = Promise.race([document.fonts.ready, new Promise((done) => setTimeout(done, 400))]);

type Props = {
  children: ReactNode;
  /** The main window: closing hides it to the tray, and it can be resized and maximized. */
  main?: boolean;
};

export function WindowMotion({ children, main = false }: Props) {
  const frame = useRef<HTMLDivElement>(null);
  const sheen = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const win = getCurrentWindow();
    const el = frame.current!;
    const root = document.documentElement;
    /** The card is gone (or not shown yet), so the next reveal plays the opening. */
    let away = true;
    /** Counts the moves: a move that was overtaken by a newer one stops there. */
    let turn = 0;
    let running: { stop: () => void }[] = [];
    const stopAll = () => {
      for (const a of running) a.stop();
      running = [];
    };
    animate(el, born, { duration: 0 });

    const enter = () => {
      if (!away) return;
      away = false;
      const mine = ++turn;
      stopAll();
      if (prefersLess()) {
        el.style.transform = "none";
        el.style.filter = "none";
        running = [animate(el, { opacity: 1 }, { duration: 0.15 })];
        return;
      }
      const body = el.querySelectorAll<HTMLElement>(":scope > .window-body > * > *");
      // Hidden now, not when each one's turn comes: none shows, then blinks away first.
      for (const part of body) {
        part.style.opacity = "0";
        part.style.transform = "translateY(12px)";
      }
      running = [
        animate(el, shown, {
          type: "spring",
          visualDuration: main ? 0.42 : 0.48,
          bounce: main ? BOUNCE.main : BOUNCE.popup,
          opacity: { duration: 0.22, ease: "easeOut" },
          filter: { duration: 0.34, ease: [0.22, 1, 0.36, 1] },
        }),
        // The window's parts settle in one after another.
        animate(
          body,
          { opacity: [0, 1], y: [12, 0] },
          { duration: 0.42, ease: [0.22, 1, 0.36, 1], delay: stagger(0.045, { startDelay: 0.06 }) },
        ),
        // A soft light sweeps across the card once.
        animate(sheen.current!, { x: ["-110%", "110%"] }, { duration: 0.9, delay: 0.14, ease: [0.45, 0, 0.2, 1] }),
      ];
      const settle = () => {
        if (mine !== turn) return;
        clearTimeout(rescue);
        stopAll();
        // At rest the card has no transform or filter: fixed menus place themselves on
        // the window, and the frosted panels blur what is behind them.
        el.style.transform = "none";
        el.style.filter = "none";
        el.style.transformOrigin = "";
        el.style.opacity = "1";
        sheen.current!.style.transform = "translateX(110%)";
        for (const part of body) {
          part.style.opacity = "";
          part.style.transform = "";
        }
      };
      void Promise.all(running).then(settle);
      // If the page was held back (a window shown while Windows paused its drawing), the
      // card still ends up shown rather than stuck halfway or invisible.
      const rescue = setTimeout(settle, 1500);
    };

    /** Plays the closing (or minimizing); false if the window was shown again meanwhile. */
    const leave = async (how: Leave) => {
      away = true;
      const mine = ++turn;
      stopAll();
      if (prefersLess()) {
        running = [animate(el, { opacity: 0 }, { duration: 0.1 })];
      } else {
        el.style.transformOrigin = how === "minimize" ? "50% 100%" : "";
        running = [
          animate(
            el,
            how === "minimize" ? sunk : closed,
            how === "minimize"
              ? { duration: 0.26, ease: [0.55, 0, 0.75, 0.2] }
              : { duration: 0.17, ease: [0.4, 0, 1, 1] },
          ),
        ];
      }
      await Promise.all(running);
      return mine === turn;
    };
    card = { enter, leave };

    let minimized = false;
    const fit = async () => {
      root.classList.toggle("maximized", await win.isMaximized());
      const now = await win.isMinimized();
      if (minimized && !now) {
        // Back from the taskbar: wake the page's drawing (see `wake_page`), then open.
        await invoke("wake_page").catch(() => {});
        enter();
      }
      minimized = now;
    };
    void fit();

    const open = () => void fontsReady.then(enter);
    const stops = [
      win.listen("window-reveal", open),
      // Back from the taskbar, or clicked: show it even if no reveal came.
      win.onFocusChanged(({ payload: focused }) => {
        if (focused) open();
      }),
      win.onResized(() => void fit()),
    ];
    const onVisible = () => {
      if (document.visibilityState === "visible") open();
    };
    document.addEventListener("visibilitychange", onVisible);
    if (main) {
      // Closed by the system (Alt+F4, the taskbar): OmniDM keeps running in the tray.
      stops.push(
        win.onCloseRequested(async (event) => {
          event.preventDefault();
          if (await leave("close")) await win.hide();
        }),
      );
    }
    // Listening first, then looking: a window shown in between is caught by one or the other.
    void stops[0].then(() => win.isVisible()).then((visible) => {
      if (visible && document.visibilityState === "visible") open();
    });

    return () => {
      card = null;
      stopAll();
      document.removeEventListener("visibilitychange", onVisible);
      for (const stop of stops) void stop.then((f) => f());
    };
  }, [main]);

  return (
    <div className="window-stage">
      <div ref={frame} className="window-frame" style={BORN_STYLE}>
        <div className="window-body">{children}</div>
        <div aria-hidden className="window-edge" />
        <div aria-hidden className="window-sheen-clip">
          <div ref={sheen} className="window-sheen" />
        </div>
      </div>
      {main && <ResizeEdges />}
    </div>
  );
}

/** The see-through space around the main window's card: drag it to resize the window. */
function ResizeEdges() {
  const edges: [ResizeDirection, string][] = [
    ["North", "top-0 inset-x-5 h-5 cursor-n-resize"],
    ["South", "bottom-0 inset-x-5 h-5 cursor-s-resize"],
    ["West", "left-0 inset-y-5 w-5 cursor-w-resize"],
    ["East", "right-0 inset-y-5 w-5 cursor-e-resize"],
    ["NorthWest", "top-0 left-0 size-5 cursor-nw-resize"],
    ["NorthEast", "top-0 right-0 size-5 cursor-ne-resize"],
    ["SouthWest", "bottom-0 left-0 size-5 cursor-sw-resize"],
    ["SouthEast", "bottom-0 right-0 size-5 cursor-se-resize"],
  ];
  return (
    <div className="window-resize">
      {edges.map(([direction, place]) => (
        <div
          key={direction}
          className={`absolute ${place}`}
          onMouseDown={(event) => {
            if (event.button === 0) void getCurrentWindow().startResizeDragging(direction);
          }}
        />
      ))}
    </div>
  );
}
