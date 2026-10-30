// SPDX-License-Identifier: GPL-3.0-or-later

import type { IconName } from "../components/Icon";

/** An icon for a category, by its usual names. */
export function categoryIcon(name: string): IconName {
  const n = name.toLowerCase();
  if (/program|software|app/.test(n)) return "app-window";
  if (/video|movie|film/.test(n)) return "clapperboard";
  if (/music|audio|sound/.test(n)) return "music";
  if (/doc|book|pdf/.test(n)) return "file-text";
  if (/image|picture|photo/.test(n)) return "image";
  if (/compress|archive|zip|rar/.test(n)) return "archive";
  return "tag";
}

/** The tinted square behind a category's icon, coloured like its files' tags. */
export function categoryTint(icon: IconName): string {
  switch (icon) {
    case "app-window":
      return "bg-[#4a6cff]/15 text-[#8ea4ff]";
    case "clapperboard":
      return "bg-[#9b6bff]/[0.18] text-[#c4a8ff]";
    case "music":
      return "bg-[#2fd3c4]/[0.17] text-[#6fe3d8]";
    case "file-text":
      return "bg-[#e5484d]/[0.17] text-[#ff9a9e]";
    case "image":
      return "bg-[#2fd3a0]/[0.17] text-[#7fe6c4]";
    case "archive":
      return "bg-[#ffb224]/[0.17] text-[#ffc95c]";
    default:
      return "bg-[#ffb224]/[0.17] text-[#ffc95c]";
  }
}
