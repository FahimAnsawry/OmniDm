// SPDX-License-Identifier: GPL-3.0-or-later

import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { CapturePrompt } from "./components/CapturePrompt";
import { DetailsWindow } from "./components/DetailsWindow";
import { MediaWindow } from "./components/MediaWindow";
import { WindowMotion } from "./components/WindowMotion";
import "./index.css";

// A capture prompt's window is labelled `capture-<download id>`, a details window
// `details-<download id>`, a video window `media-<offer>`; the main window shows the list.
const label = getCurrentWindow().label;
const capture = /^capture-(\d+)$/.exec(label);
const details = /^details-(\d+)$/.exec(label);
const media = /^media-(\d+)$/.exec(label);

// The details window has the system's title bar, which opens and closes it; the others
// draw their own and animate themselves on a see-through window.
if (!details) document.documentElement.classList.add("see-through");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {capture ? (
      <WindowMotion>
        <CapturePrompt id={Number(capture[1])} />
      </WindowMotion>
    ) : details ? (
      <DetailsWindow id={Number(details[1])} />
    ) : media ? (
      <WindowMotion>
        <MediaWindow offer={Number(media[1])} />
      </WindowMotion>
    ) : (
      <WindowMotion hideOnClose>
        <App />
      </WindowMotion>
    )}
  </React.StrictMode>,
);
