// SPDX-License-Identifier: GPL-3.0-or-later

// The connection to the OmniDM app through the native messaging host (plan §8, §10):
// requests with replies matched by id, answers to the app's own requests, and the
// connection state shown in the popup.

import type { Envelope } from '../../protocol/ts/Envelope';
import type { Hello } from '../../protocol/ts/Hello';
import type { Message } from '../../protocol/ts/Message';

export type LinkStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  /** The host ran but could not reach the app. */
  | 'notRunning'
  /** The host is not registered with this browser. */
  | 'notInstalled';

/** The parts of `runtime.Port` the link uses. */
export interface PortLike {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: { addListener(callback: (message: unknown) => void): void };
  onDisconnect: { addListener(callback: () => void): void };
}

export interface LinkOptions {
  /** Opens a new port to the host (`runtime.connectNative`). */
  connect: () => PortLike;
  /** Why the last port closed (`runtime.lastError`), read when it does. */
  lastError: () => string | undefined;
  hello: Hello;
  /** Answers a request the app sends (such as `cookiesRequest`). */
  onRequest?: (envelope: Envelope) => Promise<Message | undefined>;
  newId?: () => string;
}

const HELLO_TIMEOUT_MS = 10_000;

interface Waiting {
  resolve: (reply: Envelope) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class NativeLink {
  status: LinkStatus = 'disconnected';
  private port?: PortLike;
  private opening?: Promise<boolean>;
  private waiting = new Map<string, Waiting>();
  private statusListeners = new Set<(status: LinkStatus) => void>();

  constructor(private readonly options: LinkOptions) {}

  /** Calls `listener` whenever the connection state changes. */
  onStatus(listener: (status: LinkStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  /** Connects and says hello; `true` once the app has answered. */
  open(): Promise<boolean> {
    if (this.status === 'connected') return Promise.resolve(true);
    this.opening ??= this.connectAndGreet().finally(() => {
      this.opening = undefined;
    });
    return this.opening;
  }

  /** Sends `message` and waits up to `timeoutMs` for the app's reply. */
  request(message: Message, timeoutMs: number): Promise<Envelope> {
    // Connected: send at once, so messages leave in the order they were requested.
    if (this.status === 'connected' && this.port) {
      return this.send(this.port, message, timeoutMs);
    }
    return this.open().then((connected) => {
      if (!connected || !this.port) {
        throw new Error(`OmniDM is not available (${this.status})`);
      }
      return this.send(this.port, message, timeoutMs);
    });
  }

  close(): void {
    const port = this.port;
    this.port = undefined;
    port?.disconnect();
    this.failWaiting('disconnected');
    this.setStatus('disconnected');
  }

  private async connectAndGreet(): Promise<boolean> {
    this.setStatus('connecting');
    let port: PortLike;
    try {
      port = this.options.connect();
    } catch {
      this.setStatus('notInstalled');
      return false;
    }
    this.port = port;
    port.onMessage.addListener((message) => this.receive(port, message as Envelope));
    port.onDisconnect.addListener(() => this.dropped(port));
    try {
      const ack = await this.send(
        port,
        { type: 'hello', payload: this.options.hello },
        HELLO_TIMEOUT_MS,
      );
      if (ack.type !== 'helloAck') throw new Error(`unexpected ${ack.type}`);
      this.setStatus('connected');
      return true;
    } catch {
      // A dropped port has already set the state; anything else means the app is unusable.
      if (this.port === port) {
        this.port = undefined;
        port.disconnect();
        this.setStatus('notRunning');
      }
      return false;
    }
  }

  private send(port: PortLike, message: Message, timeoutMs: number): Promise<Envelope> {
    const id = this.newId();
    return new Promise<Envelope>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        reject(new Error(`no answer within ${timeoutMs / 1000} s`));
      }, timeoutMs);
      this.waiting.set(id, { resolve, reject, timer });
      port.postMessage({ v: 1, id, ...message });
    });
  }

  private receive(port: PortLike, envelope: Envelope): void {
    const waiting = envelope.re === undefined ? undefined : this.waiting.get(envelope.re);
    if (waiting && envelope.re !== undefined) {
      clearTimeout(waiting.timer);
      this.waiting.delete(envelope.re);
      waiting.resolve(envelope);
      return;
    }
    const handler = this.options.onRequest;
    if (!handler) return;
    void handler(envelope).then((reply) => {
      if (reply && this.port === port) {
        port.postMessage({ v: 1, id: this.newId(), re: envelope.id, ...reply });
      }
    });
  }

  private dropped(port: PortLike): void {
    if (this.port !== port) return;
    this.port = undefined;
    const reason = this.options.lastError() ?? '';
    this.failWaiting('disconnected');
    this.setStatus(
      /not found|no such native application|forbidden/i.test(reason) ? 'notInstalled' : 'notRunning',
    );
  }

  private failWaiting(why: string): void {
    for (const [id, waiting] of this.waiting) {
      clearTimeout(waiting.timer);
      waiting.reject(new Error(why));
      this.waiting.delete(id);
    }
  }

  private setStatus(status: LinkStatus): void {
    if (this.status === status) return;
    this.status = status;
    this.statusListeners.forEach((listener) => listener(status));
  }

  private newId(): string {
    return this.options.newId?.() ?? crypto.randomUUID();
  }
}
