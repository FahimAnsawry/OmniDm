// SPDX-License-Identifier: GPL-3.0-or-later

// Capture settings, kept in the extension's local storage.

export interface CaptureSettings {
  /** Send downloads to OmniDm at all. */
  enabled: boolean;
  /** Smaller downloads (when their size is known) stay in the browser. 0 = no minimum. */
  minSize: number;
  /** File extensions to capture, without dots; empty = every type. */
  fileTypes: string[];
  /** Sites (and their subdomains) whose downloads stay in the browser. */
  excludedSites: string[];
  /** Capture downloads from private windows too. */
  capturePrivate: boolean;
  /** Put a Download button on videos OmniDm found on a page. */
  videoButton: boolean;
}

export const DEFAULT_SETTINGS: CaptureSettings = {
  enabled: true,
  minSize: 0,
  fileTypes: [],
  excludedSites: [],
  capturePrivate: false,
  videoButton: true,
};
