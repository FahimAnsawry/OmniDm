// SPDX-License-Identifier: GPL-3.0-or-later

// Links collected from a page for "Download all links" and "Download selected links".

/** Each http(s) link once, in page order, without fragments. */
export function httpLinks(hrefs: string[]): string[] {
  const seen = new Set<string>();
  const links: string[] = [];
  for (const href of hrefs) {
    let url: URL;
    try {
      url = new URL(href);
    } catch {
      continue;
    }
    if (url.protocol !== 'http:' && url.protocol !== 'https:') continue;
    url.hash = '';
    const text = url.toString();
    if (!seen.has(text)) {
      seen.add(text);
      links.push(text);
    }
  }
  return links;
}

/**
 * Runs in the page (through `scripting.executeScript`) when the user picks a menu item:
 * returns the links on the page, or only those inside the selection.
 */
export function collectLinks(selectedOnly: boolean): string[] {
  const anchors = Array.from(document.querySelectorAll<HTMLAnchorElement>('a[href]'));
  if (!selectedOnly) return anchors.map((a) => a.href);
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0) return [];
  return anchors.filter((a) => selection.containsNode(a, true)).map((a) => a.href);
}
