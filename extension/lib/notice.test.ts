// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import { badgeFor, badgeForTab, shouldOpenSetup } from './notice';

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

  it('shows nothing while connected or connecting', () => {
    for (const status of ['connected', 'connecting', 'disconnected'] as const) {
      expect(badgeFor(status).text).toBe('');
      expect(badgeFor(status).title).toBe('OmniDm');
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

describe('badgeForTab', () => {
  it('counts videos unless the app is missing', () => {
    expect(badgeForTab('connected', 0).text).toBe('');
    expect(badgeForTab('connected', 2)).toMatchObject({
      text: '2',
      title: 'OmniDm found 2 videos on this page',
    });
    expect(badgeForTab('disconnected', 1).title).toBe('OmniDm found a video on this page');
    expect(badgeForTab('notInstalled', 3).text).toBe('!');
    expect(badgeForTab('notRunning', 3).color).toBe('#d97706');
  });
});
