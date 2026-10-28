// SPDX-License-Identifier: GPL-3.0-or-later

import type { BrowserCookie } from '../../protocol/ts/BrowserCookie';

/** The subset of `cookies.Cookie` OmniDm reads. */
export interface CookieLike {
  name: string;
  value: string;
  domain: string;
  hostOnly: boolean;
  path: string;
  secure: boolean;
  httpOnly: boolean;
  expirationDate?: number;
}

/** The parts of a browser cookie OmniDm needs to send it to the right places. */
export function toBrowserCookie(cookie: CookieLike): BrowserCookie {
  const result: BrowserCookie = {
    name: cookie.name,
    value: cookie.value,
    domain: cookie.domain,
    hostOnly: cookie.hostOnly,
    path: cookie.path,
    secure: cookie.secure,
    httpOnly: cookie.httpOnly,
  };
  if (cookie.expirationDate !== undefined) result.expirationDate = cookie.expirationDate;
  return result;
}
