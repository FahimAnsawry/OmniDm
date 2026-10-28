// SPDX-License-Identifier: GPL-3.0-or-later

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Envelope } from '../../protocol/ts/Envelope';
import type { Message } from '../../protocol/ts/Message';
import { NativeLink, type PortLike } from './native';

/** A port whose other end is driven by the test. */
class FakePort implements PortLike {
  sent: Envelope[] = [];
  private messageListeners: Array<(m: unknown) => void> = [];
  private disconnectListeners: Array<() => void> = [];
  closed = false;
  onMessage = { addListener: (cb: (m: unknown) => void) => this.messageListeners.push(cb) };
  onDisconnect = { addListener: (cb: () => void) => this.disconnectListeners.push(cb) };
  postMessage(message: unknown) {
    this.sent.push(message as Envelope);
  }
  disconnect() {
    this.closed = true;
  }
  /** The app sends `message`. */
  receive(message: Envelope) {
    this.messageListeners.forEach((cb) => cb(message));
  }
  /** The host exits or the app is unreachable. */
  drop() {
    this.disconnectListeners.forEach((cb) => cb());
  }
  /** Answers the last message sent with `reply`. */
  answer(reply: Message) {
    const last = this.sent[this.sent.length - 1]!;
    this.receive({ v: 1, id: `app-${this.sent.length}`, re: last.id, ...reply });
  }
}

const hello = {
  extensionVersion: '0.1.0',
  browser: 'chrome',
  browserVersion: '156',
  protocols: [1],
};

function setup(lastError?: string) {
  const ports: FakePort[] = [];
  let next = 0;
  const link = new NativeLink({
    connect: () => {
      const port = new FakePort();
      ports.push(port);
      return port;
    },
    lastError: () => lastError,
    hello,
    newId: () => `e${++next}`,
    onRequest: async (envelope) =>
      envelope.type === 'cookiesRequest'
        ? { type: 'cookiesResponse', payload: { cookies: [] } }
        : undefined,
  });
  return { link, ports };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('NativeLink', () => {
  it('says hello first and is connected once the app answers', async () => {
    const { link, ports } = setup();
    const opened = link.open();
    expect(ports[0]!.sent[0]).toMatchObject({ v: 1, type: 'hello', payload: hello });
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    expect(await opened).toBe(true);
    expect(link.status).toBe('connected');
  });

  it('matches replies to requests by id', async () => {
    const { link, ports } = setup();
    const opened = link.open();
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await opened;

    const reply = link.request({ type: 'ping' }, 1000);
    const pingId = ports[0]!.sent[1]!.id;
    ports[0]!.receive({ v: 1, id: 'x', re: 'someone-else', type: 'pong' });
    ports[0]!.receive({ v: 1, id: 'y', re: pingId, type: 'pong' });

    expect((await reply).re).toBe(pingId);
  });

  it('gives up on a request that gets no answer', async () => {
    vi.useFakeTimers();
    const { link, ports } = setup();
    const opened = link.open();
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await opened;

    const reply = link.request({ type: 'ping' }, 10_000);
    const failed = expect(reply).rejects.toThrow(/no answer/);
    await vi.advanceTimersByTimeAsync(10_001);
    await failed;
  });

  it('reports a missing host as not installed', async () => {
    const { link, ports } = setup('Specified native messaging host not found.');
    const opened = link.open();
    ports[0]!.drop();
    expect(await opened).toBe(false);
    expect(link.status).toBe('notInstalled');
  });

  it('reports a host that exits as the app not running, failing waiting requests', async () => {
    const { link, ports } = setup('Native host has exited.');
    const opened = link.open();
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await opened;

    const reply = link.request({ type: 'ping' }, 10_000);
    ports[0]!.drop();

    await expect(reply).rejects.toThrow(/disconnected/);
    expect(link.status).toBe('notRunning');
  });

  it('reconnects for the next request after a disconnect', async () => {
    const { link, ports } = setup();
    const first = link.open();
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await first;
    ports[0]!.drop();

    const reply = link.request({ type: 'ping' }, 1000);
    await vi.waitFor(() => expect(ports.length).toBe(2));
    ports[1]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await vi.waitFor(() => expect(ports[1]!.sent.length).toBe(2));
    ports[1]!.answer({ type: 'pong' });

    expect((await reply).type).toBe('pong');
  });

  it('answers requests from the app, such as fresh cookies', async () => {
    const { link, ports } = setup();
    const opened = link.open();
    ports[0]!.answer({ type: 'helloAck', payload: { appVersion: '0.1.0', protocol: 1 } });
    await opened;

    ports[0]!.receive({
      v: 1,
      id: 'app-77',
      type: 'cookiesRequest',
      payload: { url: 'https://e.com/a' },
    });

    await vi.waitFor(() =>
      expect(ports[0]!.sent.at(-1)).toMatchObject({ re: 'app-77', type: 'cookiesResponse' }),
    );
  });
});
