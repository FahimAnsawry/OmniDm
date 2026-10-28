// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it, vi } from 'vitest';
import type { Envelope } from '../../protocol/ts/Envelope';
import type { Message } from '../../protocol/ts/Message';
import { handleCreated, type CaptureDeps, type DownloadItemLike } from './capture';
import { HeaderCache } from './header-cache';
import { DEFAULT_SETTINGS } from './settings';

const item = (over: Partial<DownloadItemLike> = {}): DownloadItemLike => ({
  id: 42,
  url: 'https://example.com/get?id=1',
  finalUrl: 'https://cdn.example.com/report.pdf',
  referrer: 'https://example.com/page',
  filename: 'C:\\Users\\me\\Downloads\\report.pdf',
  mime: 'application/pdf',
  totalBytes: 5_000_000,
  incognito: false,
  ...over,
});

type Answer = Message | Error;

function deps(answer: Answer, over: Partial<CaptureDeps> = {}) {
  const calls: string[] = [];
  const sent: Message[] = [];
  const cache = new HeaderCache();
  const d: CaptureDeps = {
    settings: async () => DEFAULT_SETTINGS,
    downloads: {
      pause: vi.fn(async (id: number) => void calls.push(`pause ${id}`)),
      resume: vi.fn(async (id: number) => void calls.push(`resume ${id}`)),
      cancel: vi.fn(async (id: number) => void calls.push(`cancel ${id}`)),
      erase: vi.fn(async (query: { id: number }) => void calls.push(`erase ${query.id}`)),
    },
    cookies: async () => [
      {
        name: 'session',
        value: 'abc',
        domain: '.example.com',
        hostOnly: false,
        path: '/',
        secure: true,
        httpOnly: true,
      },
    ],
    cache,
    request: async (message: Message): Promise<Envelope> => {
      sent.push(message);
      if (answer instanceof Error) throw answer;
      return { v: 1, id: 'a', re: 'x', ...answer } as Envelope;
    },
    now: () => 1000,
    ...over,
  };
  return { d, calls, sent, cache };
}

const accepted: Message = {
  type: 'captureResult',
  payload: { outcome: 'accepted', downloadId: 9 },
};

describe('handleCreated', () => {
  it('hands the download to OmniDm and removes the browser copy when accepted', async () => {
    const { d, calls, sent, cache } = deps(accepted);
    cache.rememberHeaders('https://example.com/get?id=1', [{ name: 'User-Agent', value: 'UA' }], 900);

    expect(await handleCreated(item(), d)).toBe('captured');

    expect(calls).toEqual(['pause 42', 'cancel 42', 'erase 42']);
    expect(sent[0]).toEqual({
      type: 'capture',
      payload: {
        url: 'https://example.com/get?id=1',
        finalUrl: 'https://cdn.example.com/report.pdf',
        referrer: 'https://example.com/page',
        headers: [{ name: 'User-Agent', value: 'UA' }],
        cookies: [expect.objectContaining({ name: 'session', value: 'abc' })],
        suggestedName: 'report.pdf',
        mime: 'application/pdf',
        size: 5_000_000,
        private: false,
        browserDownloadId: 42,
      },
    });
  });

  it('sends a POST form with its body', async () => {
    const { d, sent, cache } = deps(accepted);
    cache.rememberBody('https://example.com/get?id=1', 'POST', 'aWQ9Nw==', 900);

    await handleCreated(item(), d);

    const payload = (sent[0] as Extract<Message, { type: 'capture' }>).payload;
    expect(payload.method).toBe('POST');
    expect(payload.body).toBe('aWQ9Nw==');
  });

  it('gives the download back to the browser when OmniDm declines it', async () => {
    const { d, calls } = deps({
      type: 'captureResult',
      payload: { outcome: 'rejected', reason: 'probeFailed', detail: 'HTTP 403' },
    });

    expect(await handleCreated(item(), d)).toBe('resumed');
    expect(calls).toEqual(['pause 42', 'resume 42']);
  });

  it('gives the download back when the app cannot be reached or times out', async () => {
    const { d, calls } = deps(new Error('no answer within 10 s'));

    expect(await handleCreated(item(), d)).toBe('resumed');
    expect(calls).toEqual(['pause 42', 'resume 42']);
  });

  it('gives the download back on an unexpected answer', async () => {
    const { d, calls } = deps({ type: 'pong' });

    expect(await handleCreated(item(), d)).toBe('resumed');
    expect(calls).toEqual(['pause 42', 'resume 42']);
  });

  it('does not touch downloads the rules leave to the browser', async () => {
    const { d, calls, sent } = deps(accepted, {
      settings: async () => ({ ...DEFAULT_SETTINGS, enabled: false }),
    });

    expect(await handleCreated(item(), d)).toBe('left');
    expect(calls).toEqual([]);
    expect(sent).toEqual([]);
  });

  it('leaves a download that finished before it could be paused', async () => {
    const { d, sent } = deps(accepted);
    d.downloads.pause = async () => {
      throw new Error('Download must be in progress');
    };

    expect(await handleCreated(item(), d)).toBe('left');
    expect(sent).toEqual([]);
  });

  it('marks private-window downloads and sends no size when it is unknown', async () => {
    const { d, sent } = deps(accepted, {
      settings: async () => ({ ...DEFAULT_SETTINGS, capturePrivate: true }),
    });

    await handleCreated(item({ incognito: true, totalBytes: -1 }), d);

    const payload = (sent[0] as Extract<Message, { type: 'capture' }>).payload;
    expect(payload.private).toBe(true);
    expect(payload.size).toBeUndefined();
  });
});
