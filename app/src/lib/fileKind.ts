// SPDX-License-Identifier: GPL-3.0-or-later

// What kind of file a name is, for its icon, colour and label.

import type { IconName } from "../components/Icon";

export type FileKind = {
  /** "Disk image", "Program"… */
  label: string;
  /** The line icon drawn for it. */
  icon: IconName;
  /** The icon's colour in the main window, light enough for the dark background. */
  color: string;
};

const SUNSET = "#ff9a5c";
const BLUE = "#8ea4ff";
const VIOLET = "#b197fc";
const TEAL = "#38d9a9";
const RED = "#ff8787";
const AMBER = "#fcc419";
const PINK = "#f783ac";
const GRAY = "#8b93a9";

const KINDS: { extensions: string[]; label: string; icon: IconName; color: string }[] = [
  { extensions: ["iso", "img", "dmg", "vhd", "vhdx", "wim"], label: "Disk image", icon: "disc", color: SUNSET },
  { extensions: ["exe", "msi", "msix", "appx", "apk", "deb", "rpm", "bat", "cmd"], label: "Program", icon: "app-window", color: BLUE },
  { extensions: ["mp4", "mkv", "webm", "mov", "avi", "ts", "m4v", "wmv", "flv"], label: "Video", icon: "film", color: VIOLET },
  { extensions: ["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"], label: "Music", icon: "music", color: TEAL },
  { extensions: ["pdf"], label: "PDF document", icon: "file-text", color: RED },
  { extensions: ["doc", "docx", "odt", "rtf", "txt", "xls", "xlsx", "ppt", "pptx", "epub", "csv"], label: "Document", icon: "file-text", color: RED },
  { extensions: ["zip", "rar", "7z", "tar", "gz", "xz", "bz2", "zst", "cab"], label: "Archive", icon: "archive", color: AMBER },
  { extensions: ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "heic", "avif"], label: "Image", icon: "image", color: PINK },
];

export function fileKind(name: string): FileKind {
  const dot = name.lastIndexOf(".");
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  const known = KINDS.find((k) => k.extensions.includes(extension));
  return known ? { label: known.label, icon: known.icon, color: known.color } : { label: "File", icon: "file", color: GRAY };
}
