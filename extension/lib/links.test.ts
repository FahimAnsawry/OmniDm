// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import { toBrowserCookie } from './cookies';
import { httpLinks } from './links';

describe('httpLinks', () => {
  it('keeps each http and https link once, in page order', () => {
    expect(
      httpLinks([
        'https://e.com/a.zip',
        'javascript:void(0)',
        'mailto:me@e.com',
        'http://e.com/b.iso',
        'https://e.com/a.zip',
        'https://e.com/a.zip#part',
        '',
        'not a url',
      ]),
    ).toEqual(['https://e.com/a.zip', 'http://e.com/b.iso']);
  });
});

describe('toBrowserCookie', () => {
  it('keeps the fields OmniDm needs and drops the rest', () => {
    const fromBrowser = {
        name: 'session',
        value: 'abc',
        domain: '.example.com',
        hostOnly: false,
        path: '/',
        secure: true,
        httpOnly: true,
        session: false,
        expirationDate: 1893456000,
        sameSite: 'lax',
        storeId: '0',
    };
    expect(toBrowserCookie(fromBrowser)).toEqual({
      name: 'session',
      value: 'abc',
      domain: '.example.com',
      hostOnly: false,
      path: '/',
      secure: true,
      httpOnly: true,
      expirationDate: 1893456000,
    });
  });

  it('leaves out the expiry of a session cookie', () => {
    const fromBrowser = {
      name: 'a',
      value: '1',
      domain: 'e.com',
      hostOnly: true,
      path: '/',
      secure: false,
      httpOnly: false,
      session: true,
      sameSite: 'unspecified',
      storeId: '0',
    };
    const cookie = toBrowserCookie(fromBrowser);
    expect('expirationDate' in cookie).toBe(false);
  });
});
