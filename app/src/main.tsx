// SPDX-License-Identifier: GPL-3.0-or-later

import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { CapturePrompt } from "./components/CapturePrompt";
import "./index.css";

// A capture prompt's window is labelled `capture-<download id>`; the main window shows the list.
const capture = /^capture-(\d+)$/.exec(getCurrentWindow().label);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {capture ? <CapturePrompt id={Number(capture[1])} /> : <App />}
  </React.StrictMode>,
);
