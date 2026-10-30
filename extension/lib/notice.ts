// SPDX-License-Identifier: GPL-3.0-or-later

// The "Install OmniDM" notice (plan §9.3): a badge on the toolbar icon while the app
// can't be found or when the extension is disabled, and a setup page once, right after the
// extension is installed.

import type { LinkStatus } from './native';

/** Where to get the app. */
export const DOWNLOAD_PAGE = 'https://github.com/FahimAnsawry/OmniDm/releases';

export interface Badge {
  /** Shown on the icon; empty clears it. */
  text: string;
  color: string;
  /** The icon's tooltip. */
  title: string;
}

const NORMAL: Badge = { text: '', color: '#0284c7', title: 'OmniDM (click to disable)' };

const DISABLED: Badge = {
  text: 'OFF',
  color: '#6b7280',
  title: 'OmniDM is disabled (click to enable)',
};

/** The toolbar icon's badge for a connection state and enabled toggle. */
export function badgeFor(status: LinkStatus, enabled = true): Badge {
  if (!enabled) return DISABLED;
  switch (status) {
    case 'notInstalled':
      return {
        text: '!',
        color: '#dc2626',
        title: "OmniDM isn't installed, so downloads stay in the browser.",
      };
    case 'notRunning':
      return {
        text: '!',
        color: '#d97706',
        title: "OmniDM didn't start, so downloads stay in the browser.",
      };
    default:
      return NORMAL;
  }
}

/**
 * Opens the setup page only right after the extension was first installed (not on an
 * update or a browser update), and only when the app wasn't found.
 */
export function shouldOpenSetup(reason: string, status: LinkStatus): boolean {
  return reason === 'install' && status === 'notInstalled';
}
