// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import { encodeRequestBody, HeaderCache } from './header-cache';

describe('HeaderCache', () => {
  it('returns what was seen for a URL', () => {
    const cache = new HeaderCache();
    cache.rememberHeaders('https://e.com/a.zip', [{ name: 'Referer', value: 'https://e.com/' }], 0);
    expect(cache.lookup(['https://e.com/a.zip'], 1000)).toEqual({
      headers: [{ name: 'Referer', value: 'https://e.com/' }],
    });
  });

  it('keeps the body and method of a POST with its headers', () => {
    const cache = new HeaderCache();
    cache.rememberBody('https://e.com/form', 'POST', 'aWQ9Nw==', 0);
    cache.rememberHeaders('https://e.com/form', [{ name: 'Origin', value: 'https://e.com' }], 0);
    expect(cache.lookup(['https://e.com/form'], 10)).toEqual({
      method: 'POST',
      body: 'aWQ9Nw==',
      headers: [{ name: 'Origin', value: 'https://e.com' }],
    });
  });

  it('finds an entry by any of the URLs given (final URL after redirects)', () => {
    const cache = new HeaderCache();
    cache.rememberHeaders('https://cdn.e.com/a.zip', [{ name: 'X', value: '1' }], 0);
    expect(cache.lookup(['https://e.com/get', 'https://cdn.e.com/a.zip'], 10)?.headers).toEqual([
      { name: 'X', value: '1' },
    ]);
  });

  it('forgets entries after 30 seconds', () => {
    const cache = new HeaderCache();
    cache.rememberHeaders('https://e.com/a', [], 0);
    expect(cache.lookup(['https://e.com/a'], 29_999)).toBeDefined();
    expect(cache.lookup(['https://e.com/a'], 30_001)).toBeUndefined();
  });

  it('keeps at most 200 entries, dropping the oldest', () => {
    const cache = new HeaderCache();
    for (let i = 0; i < 201; i++) {
      cache.rememberHeaders(`https://e.com/${i}`, [], i);
    }
    expect(cache.size).toBe(200);
    expect(cache.lookup(['https://e.com/0'], 300)).toBeUndefined();
    expect(cache.lookup(['https://e.com/200'], 300)).toBeDefined();
  });

  it('never keeps cookie headers (cookies come from the cookies API)', () => {
    const cache = new HeaderCache();
    cache.rememberHeaders(
      'https://e.com/a',
      [
        { name: 'Cookie', value: 'session=abc' },
        { name: 'User-Agent', value: 'Mozilla/5.0' },
      ],
      0,
    );
    expect(cache.lookup(['https://e.com/a'], 1)?.headers).toEqual([
      { name: 'User-Agent', value: 'Mozilla/5.0' },
    ]);
  });
});

describe('encodeRequestBody', () => {
  it('turns form fields into urlencoded form data, base64-encoded', () => {
    const body = encodeRequestBody({ formData: { id: ['7'], agree: ['yes'], 'a b': ['x&y'] } });
    expect(atob(body!)).toBe('id=7&agree=yes&a+b=x%26y');
  });

  it('joins raw body parts', () => {
    const encoder = new TextEncoder();
    const body = encodeRequestBody({
      raw: [{ bytes: encoder.encode('{"a":').buffer }, { bytes: encoder.encode('1}').buffer }],
    });
    expect(atob(body!)).toBe('{"a":1}');
  });

  it('gives nothing for a missing or unreadable body', () => {
    expect(encodeRequestBody(undefined)).toBeUndefined();
    expect(encodeRequestBody({ error: 'Unknown error.' })).toBeUndefined();
    expect(encodeRequestBody({ raw: [{ file: 'C:/upload.bin' }] })).toBeUndefined();
  });
});
