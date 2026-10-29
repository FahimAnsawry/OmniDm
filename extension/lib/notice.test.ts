// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import { badgeFor, shouldOpenSetup } from './notice';

describe('badgeFor', () => {
  it('marks a missing app in red', () => {
    const badge = badgeFor('notInstalled');
    expect(badge.text).toBe('!');
    expect(badge.color).toBe('#dc2626');
    expect(badge.title).toMatch(/isn't installed/);
  });

  it('marks an app that did not start in amber', () => {
    const badge = badgeFor('notRunning');
    expect(badge.text).toBe('!');
    expect(badge.color).toBe('#d97706');
  });

  it('shows nothing while connected or connecting when enabled', () => {
    for (const status of ['connected', 'connecting', 'disconnected'] as const) {
      expect(badgeFor(status).text).toBe('');
      expect(badgeFor(status).title).toBe('OmniDm (click to disable)');
    }
  });

  it('shows OFF when disabled regardless of connection status', () => {
    for (const status of ['connected', 'notInstalled', 'notRunning'] as const) {
      const badge = badgeFor(status, false);
      expect(badge.text).toBe('OFF');
      expect(badge.color).toBe('#6b7280');
      expect(badge.title).toBe('OmniDm is disabled (click to enable)');
    }
  });
});

describe('shouldOpenSetup', () => {
  it('opens once, after a first install, when the app is missing', () => {
    expect(shouldOpenSetup('install', 'notInstalled')).toBe(true);
  });

  it('stays closed when the app is there, it did not start, or on updates', () => {
    expect(shouldOpenSetup('install', 'connected')).toBe(false);
    expect(shouldOpenSetup('install', 'notRunning')).toBe(false);
    expect(shouldOpenSetup('update', 'notInstalled')).toBe(false);
    expect(shouldOpenSetup('chrome_update', 'notInstalled')).toBe(false);
  });
});
