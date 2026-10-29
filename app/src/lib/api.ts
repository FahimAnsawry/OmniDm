// SPDX-License-Identifier: GPL-3.0-or-later

// The app's Tauri commands, typed.

import { invoke } from "@tauri-apps/api/core";
import type { AskAbout } from "../../../protocol/ts/AskAbout";
import type { CaptureAnswer } from "../../../protocol/ts/CaptureAnswer";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { DownloadSummary } from "../../../protocol/ts/DownloadSummary";
import type { QueueSummary } from "../../../protocol/ts/QueueSummary";

export type NewDownload = {
  url: string;
  /** Blank: the server's name. */
  fileName?: string;
  folder: string;
  connections?: number;
  /** Put in this queue instead of starting now. */
  queue?: string;
};

export type Place = { category?: string; folder: string };

export const api = {
  listDownloads: () => invoke<DownloadSummary[]>("list_downloads"),
  listQueues: () => invoke<QueueSummary[]>("list_queues"),
  listCategories: () => invoke<CategorySummary[]>("list_categories"),
  pause: (ids: number[]) => invoke<void>("pause_downloads", { ids }),
  resume: (ids: number[]) => invoke<void>("resume_downloads", { ids }),
  remove: (ids: number[], deleteFiles: boolean) =>
    invoke<void>("delete_downloads", { ids, deleteFiles }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<number>("resume_all"),
  startQueue: (name: string) => invoke<void>("start_queue", { name }),
  stopQueue: (name: string) => invoke<void>("stop_queue", { name }),
  openFile: (id: number) => invoke<void>("open_file", { id }),
  showInFolder: (id: number) => invoke<void>("show_in_folder", { id }),
  cancelAfterQueue: () => invoke<boolean>("cancel_after_queue"),
  addDownload: (download: NewDownload) => invoke<number>("add_download", { download }),
  suggestPlace: (url: string, fileName?: string) =>
    invoke<Place>("suggest_place", { url, fileName: fileName || null }),
  capturePrompt: (id: number) => invoke<AskAbout | null>("capture_prompt", { id }),
  answerCapture: (answer: CaptureAnswer) => invoke<void>("answer_capture", { answer }),
  cancelCapture: (id: number) => invoke<void>("cancel_capture", { id }),
  setAskBeforeDownload: (ask: boolean) => invoke<void>("set_ask_before_download", { ask }),
};

/** Tauri commands reject with the Rust error string. */
export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
