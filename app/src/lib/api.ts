// SPDX-License-Identifier: GPL-3.0-or-later

// The app's Tauri commands, typed.

import { invoke } from "@tauri-apps/api/core";
import type { AskAbout } from "../../../protocol/ts/AskAbout";
import type { CaptureAnswer } from "../../../protocol/ts/CaptureAnswer";
import type { CategoryEdit } from "../../../protocol/ts/CategoryEdit";
import type { CategorySummary } from "../../../protocol/ts/CategorySummary";
import type { DownloadDetails } from "../../../protocol/ts/DownloadDetails";
import type { DownloadSummary } from "../../../protocol/ts/DownloadSummary";
import type { MediaInfo } from "../../../protocol/ts/MediaInfo";
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

/** A quality, by its picture height and bandwidth. */
export type QualityPick = { height: number | null; bandwidth: number };

/** A video (HLS or DASH) to download, as the quality window chose it. */
export type NewMedia = {
  url: string;
  /** The file's name, without an extension. */
  title: string;
  folder: string;
  /** Absent: the best. */
  quality?: QualityPick;
  /** Absent: the one the video suggests. */
  audio?: { name: string; language: string | null };
  queue?: string;
};

export type Place = { category?: string; folder: string };

export type AppSettings = {
  downloadsFolder: string;
  /** Downloads outside queues that run at once. */
  maxActive: number;
  /** Connections one download may use. */
  maxConnections: number;
  askBeforeDownload: boolean;
  notifyFinished: boolean;
  clipboardMonitor: boolean;
  startWithWindows: boolean;
};

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
  openExtensionGuide: () => invoke<void>("open_extension_guide"),
  cancelAfterQueue: () => invoke<boolean>("cancel_after_queue"),
  addDownload: (download: NewDownload) => invoke<number>("add_download", { download }),
  lookAtMedia: (url: string) => invoke<MediaInfo>("look_at_media", { url }),
  addMedia: (media: NewMedia) => invoke<number>("add_media", { media }),
  mediaOffer: (offer: number) =>
    invoke<{ url: string; info: MediaInfo; quality: QualityPick | null }>("media_offer", { offer }),
  acceptMedia: (accepted: Omit<NewMedia, "url"> & { offer: number }) =>
    invoke<number>("accept_media", { accepted }),
  suggestPlace: (url: string, fileName?: string) =>
    invoke<Place>("suggest_place", { url, fileName: fileName || null }),
  capturePrompt: (id: number) => invoke<AskAbout | null>("capture_prompt", { id }),
  answerCapture: (answer: CaptureAnswer) => invoke<void>("answer_capture", { answer }),
  cancelCapture: (id: number) => invoke<void>("cancel_capture", { id }),
  setAskBeforeDownload: (ask: boolean) => invoke<void>("set_ask_before_download", { ask }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<AppSettings>("save_settings", { settings }),
  createCategory: (edit: CategoryEdit) => invoke<number>("create_category", { edit }),
  editCategory: (id: number, edit: CategoryEdit) => invoke<void>("edit_category", { id, edit }),
  deleteCategory: (id: number) => invoke<void>("delete_category", { id }),
  moveCategory: (id: number, index: number) => invoke<void>("move_category", { id, index }),
  openDetails: (id: number) => invoke<void>("open_details", { id }),
  details: (id: number) => invoke<DownloadDetails | null>("download_details", { id }),
};

/** Tauri commands reject with the Rust error string. */
export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
