// SPDX-License-Identifier: GPL-3.0-or-later

// What kind of file a name is, for its icon's tag and colour.

export type FileKind = {
  /** The tag on the icon: the extension, upper case, at most 4 letters. */
  tag: string;
  /** "Disk image", "Program"… */
  label: string;
  /** The tag's colour; two for a gradient. */
  colors: [string, string];
};

const SUNSET: [string, string] = ["#ea580c", "#be123c"];
const BLUE: [string, string] = ["#3b5bdb", "#2b44b8"];
const VIOLET: [string, string] = ["#7c3aed", "#5b21b6"];
const TEAL: [string, string] = ["#0d9488", "#0f766e"];
const RED: [string, string] = ["#e03131", "#b91c1c"];
const AMBER: [string, string] = ["#d97706", "#b45309"];
const PINK: [string, string] = ["#db2777", "#9d174d"];
const GRAY: [string, string] = ["#4b5563", "#374151"];

const KINDS: { extensions: string[]; label: string; colors: [string, string] }[] = [
  { extensions: ["iso", "img", "dmg", "vhd", "vhdx", "wim"], label: "Disk image", colors: SUNSET },
  { extensions: ["exe", "msi", "msix", "appx", "apk", "deb", "rpm", "bat", "cmd"], label: "Program", colors: BLUE },
  { extensions: ["mp4", "mkv", "webm", "mov", "avi", "ts", "m4v", "wmv", "flv"], label: "Video", colors: VIOLET },
  { extensions: ["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"], label: "Music", colors: TEAL },
  { extensions: ["pdf"], label: "PDF document", colors: RED },
  { extensions: ["doc", "docx", "odt", "rtf", "txt", "xls", "xlsx", "ppt", "pptx", "epub", "csv"], label: "Document", colors: RED },
  { extensions: ["zip", "rar", "7z", "tar", "gz", "xz", "bz2", "zst", "cab"], label: "Archive", colors: AMBER },
  { extensions: ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "heic", "avif"], label: "Image", colors: PINK },
];

export function fileKind(name: string): FileKind {
  const dot = name.lastIndexOf(".");
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  const known = KINDS.find((k) => k.extensions.includes(extension));
  const tag = /^[a-z0-9]{1,4}$/.test(extension) ? extension.toUpperCase() : "FILE";
  return known ? { tag, label: known.label, colors: known.colors } : { tag, label: "File", colors: GRAY };
}
