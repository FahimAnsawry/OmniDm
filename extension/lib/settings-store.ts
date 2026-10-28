// SPDX-License-Identifier: GPL-3.0-or-later

// The capture settings in the extension's storage, shared by the background, the popup
// and the options page.

import { storage } from '#imports';
import { DEFAULT_SETTINGS, type CaptureSettings } from './settings';

export const settingsItem = storage.defineItem<CaptureSettings>('local:settings', {
  fallback: DEFAULT_SETTINGS,
});

/** The stored settings, with defaults for anything missing. */
export async function loadSettings(): Promise<CaptureSettings> {
  return { ...DEFAULT_SETTINGS, ...(await settingsItem.getValue()) };
}
