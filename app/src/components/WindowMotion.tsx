// SPDX-License-Identifier: GPL-3.0-or-later

// Opens and closes a window the way macOS does: its content grows in from slightly
// smaller and fades in, and shrinks and fades out before the window closes, hides or
// minimizes. The window is see-through, so the desktop shows around the content; its
// shadow is on only while the content is fully there.

import { type ReactNode, useEffect } from "react";
import { MotionConfig, motion, useAnimate } from "motion/react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const shown = { opacity: 1, scale: 1 };
const gone = { opacity: 0, scale: 0.96 };

/** The window's content, while a `WindowMotion` is on the page. */
let content: { enter: () => void; leave: () => Promise<void> } | null = null;

/** Shrinks the content away, then closes the window. */
export async function closeWindow() {
  await content?.leave();
  await getCurrentWindow().close();
}

/** Shrinks the content away, then closes the window without asking its page. */
export async function destroyWindow() {
  await content?.leave();
  const win = getCurrentWindow();
  await win.destroy().catch(() => win.close());
}

/** Shrinks the content away, then minimizes the window; it grows back when restored. */
export async function minimizeWindow() {
  await content?.leave();
  await getCurrentWindow().minimize();
}

/** Grows the content back in, when a close was called off. */
export function showWindowContent() {
  content?.enter();
}

type Props = {
  children: ReactNode;
  /** Closing hides the window instead (the main window, which lives on in the tray). */
  hideOnClose?: boolean;
};

export function WindowMotion({ children, hideOnClose = false }: Props) {
  const [scope, animate] = useAnimate<HTMLDivElement>();

  useEffect(() => {
    const win = getCurrentWindow();
    let away = false;
    const enter = () => {
      away = false;
      void animate(scope.current, shown, { type: "spring", duration: 0.3, bounce: 0.15 }).then(() => {
        if (!away) void win.setShadow(true);
      });
    };
    const leave = async () => {
      away = true;
      void win.setShadow(false);
      await animate(scope.current, gone, { duration: 0.15, ease: "easeIn" });
    };
    content = { enter, leave };
    enter();

    const stops = [
      // Shown again from the tray or the taskbar after hiding or minimizing.
      win.onFocusChanged(({ payload: focused }) => {
        if (focused && away) enter();
      }),
    ];
    if (hideOnClose) {
      // Closed by the system (Alt+F4, the taskbar): the app keeps running in the tray.
      stops.push(
        win.onCloseRequested(async (event) => {
          event.preventDefault();
          await leave();
          await win.hide();
        }),
      );
    }
    return () => {
      content = null;
      for (const stop of stops) void stop.then((f) => f());
    };
  }, [animate, scope, hideOnClose]);

  return (
    <MotionConfig reducedMotion="user">
      <motion.div ref={scope} initial={gone}>
        {children}
      </motion.div>
    </MotionConfig>
  );
}
