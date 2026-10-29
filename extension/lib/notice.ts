// SPDX-License-Identifier: GPL-3.0-or-later

// The "Install OmniDm" notice (plan §9.3): a badge on the toolbar icon while the app
// can't be found, and a setup page once, right after the extension is installed. Otherwise
// the badge counts the videos found on the page.

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

const NORMAL: Badge = { text: '', color: '#0284c7', title: 'OmniDm' };

/** The toolbar icon's badge for a connection state. */
export function badgeFor(status: LinkStatus): Badge {
  switch (status) {
    case 'notInstalled':
      return {
        text: '!',
        color: '#dc2626',
        title: "OmniDm isn't installed, so downloads stay in the browser. Click for help.",
      };
    case 'notRunning':
      return {
        text: '!',
        color: '#d97706',
        title: "OmniDm didn't start, so downloads stay in the browser. Click to try again.",
      };
    default:
      return NORMAL;
  }
}

/** The badge on one tab: the notice while the app can't be found, else how many videos
 * on the page can be downloaded. */
export function badgeForTab(status: LinkStatus, videos: number): Badge {
  const notice = badgeFor(status);
  if (notice.text || videos === 0) return notice;
  return {
    text: String(videos),
    color: '#0284c7',
    title: videos === 1 ? 'OmniDm found a video on this page' : `OmniDm found ${videos} videos on this page`,
  };
}

/**
 * Opens the setup page only right after the extension was first installed (not on an
 * update or a browser update), and only when the app wasn't found.
 */
export function shouldOpenSetup(reason: string, status: LinkStatus): boolean {
  return reason === 'install' && status === 'notInstalled';
}
