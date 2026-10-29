// SPDX-License-Identifier: GPL-3.0-or-later

export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${value} B`;
  return `${value < 10 ? value.toFixed(2) : value < 100 ? value.toFixed(1) : value.toFixed(0)} ${units[unit]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/** "45 s", "12 min", "3 h 20 min". */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  const s = Math.ceil(seconds);
  if (s < 60) return `${s} s`;
  const minutes = Math.round(s / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
  return `${Math.round(hours / 24)} days`;
}

const timeOnly = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dateOnly = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const dateWithYear = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const full = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** Today's times as "14:05", this year's as "3 Oct", older with the year. */
export function formatAdded(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();
  if (date.toDateString() === now.toDateString()) return timeOnly.format(date);
  if (date.getFullYear() === now.getFullYear()) return dateOnly.format(date);
  return dateWithYear.format(date);
}

export function formatFullDate(unixSeconds: number): string {
  return full.format(new Date(unixSeconds * 1000));
}
