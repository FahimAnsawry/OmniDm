// SPDX-License-Identifier: GPL-3.0-or-later

// Request headers and POST bodies seen for recent requests, so a download can be handed
// to OmniDm with them (plan §9.4). Nothing is stored: entries live 30 s, 200 at most.

import type { Header } from '../../protocol/ts/Header';

const LIFETIME_MS = 30_000;
const MAX_ENTRIES = 200;

export interface CachedRequest {
  headers?: Header[];
  method?: string;
  /** Base64-encoded POST body. */
  body?: string;
}

interface Entry extends CachedRequest {
  seenAt: number;
}

export class HeaderCache {
  private entries = new Map<string, Entry>();

  get size(): number {
    return this.entries.size;
  }

  rememberHeaders(url: string, headers: Header[], now: number): void {
    // Cookies come from the cookies API, with their rules; never from a raw header.
    const kept = headers.filter((h) => h.name.toLowerCase() !== 'cookie');
    this.update(url, { headers: kept }, now);
  }

  rememberBody(url: string, method: string, body: string | undefined, now: number): void {
    this.update(url, { method, body }, now);
  }

  /** What was seen for the first of `urls` that has a fresh entry. */
  lookup(urls: string[], now: number): CachedRequest | undefined {
    for (const url of urls) {
      const entry = this.entries.get(url);
      if (entry && now - entry.seenAt <= LIFETIME_MS) {
        const { seenAt: _, ...request } = entry;
        return request;
      }
    }
    return undefined;
  }

  private update(url: string, part: CachedRequest, now: number): void {
    const merged: Entry = { ...this.entries.get(url), ...part, seenAt: now };
    // Re-inserting moves the entry to the end, so the oldest is always first.
    this.entries.delete(url);
    this.entries.set(url, merged);
    for (const [key, entry] of this.entries) {
      if (this.entries.size <= MAX_ENTRIES && now - entry.seenAt <= LIFETIME_MS) break;
      this.entries.delete(key);
    }
  }
}

/** The parts of `webRequest`'s request body OmniDm can resend. */
export interface RequestBodyLike {
  error?: string;
  formData?: Record<string, string[]>;
  raw?: Array<{ bytes?: ArrayBuffer; file?: string }>;
}

/** A request body as base64, or undefined when there is none or it can't be resent. */
export function encodeRequestBody(body: RequestBodyLike | undefined): string | undefined {
  if (!body || body.error) return undefined;
  let bytes: Uint8Array | undefined;
  if (body.formData) {
    const form = new URLSearchParams();
    for (const [name, values] of Object.entries(body.formData)) {
      for (const value of values) form.append(name, value);
    }
    bytes = new TextEncoder().encode(form.toString());
  } else if (body.raw) {
    if (body.raw.some((part) => !part.bytes)) return undefined;
    const parts = body.raw.map((part) => new Uint8Array(part.bytes!));
    bytes = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
    let at = 0;
    for (const part of parts) {
      bytes.set(part, at);
      at += part.length;
    }
  }
  if (!bytes) return undefined;
  let text = '';
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text);
}
