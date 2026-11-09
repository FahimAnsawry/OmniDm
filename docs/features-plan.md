# OmniDM: Features Plan (social media)

Written 2026-11-09. **Status: Part A built (2026-11-09), waiting for the manual test on each real site before release.** Torrent download and torrent search were dropped on 2026-11-09 (see Decided).

| Part | What the user gets | Size |
|---|---|---|
| **A. Social media** | Download videos from Facebook, Instagram, TikTok and X | Small to medium |

---

## Part A: Facebook, Instagram, TikTok, X

**Goal:** the user opens a TikTok video or an X post, a Download button shows on the video, they pick a quality, and it downloads. Pasting the link into Add download works too.

OmniDM already reads YouTube through yt-dlp (`crates/omnidm-service/src/ytdlp.rs`). yt-dlp also supports these four sites, so most of the work is making that code general.

### Steps

**A1. Let yt-dlp read any supported site** (`crates/omnidm-service/src/ytdlp.rs`, `crates/omnidm-service/src/manager/media.rs`)
- Replace `is_youtube` with a check against a list of supported sites: `facebook.com`, `fb.watch`, `instagram.com`, `tiktok.com`, `vm.tiktok.com`, `x.com`, `twitter.com` (plus YouTube).
- Match exact hosts or their subdomains only. A look-alike such as `tiktok.com.evil.example` must be refused.
- The fast YouTube shortcut (`direct_player`, the player API) stays YouTube-only. Other sites always go through yt-dlp.
- Error messages name the right site, for example "TikTok: This video is private", not "YouTube: …" (`problem()`).
- `StreamKind::YouTube` becomes a general yt-dlp kind that carries the site name. This changes `crates/omnidm-ipc/src/protocol.rs`; the tests regenerate `protocol/ts/`.

**A2. Pass along what each site needs**
- TikTok only serves a video with cookies that yt-dlp gives for each format (the format's `cookies` field). Today `format_of` only keeps `http_headers`, so it must keep these cookies too.
- Instagram and Facebook often need the user to be logged in. The extension already sends the page's cookies to yt-dlp.
- A link pasted into Add download has no cookies. When the site asks for a login, show: "Open this post in your browser and use the Download button there."
- `format_of` accepts only `http`/`https` formats today. That is enough at first, since X also offers plain MP4 formats. Reading HLS formats through the existing HLS code can come later.

**A3. Find the videos in the browser** (`extension/lib/media.ts`, `extension/entrypoints/background.ts`)
- Recognise post links by their address, the same way `youtubeVideo()` recognises YouTube:
  - TikTok: `tiktok.com/@user/video/<id>`, `vm.tiktok.com/<code>`
  - Instagram: `instagram.com/p/<id>`, `instagram.com/reel/<id>`
  - X: `x.com/<user>/status/<id>`, `twitter.com/<user>/status/<id>`
  - Facebook: `facebook.com/watch?v=<id>`, `facebook.com/reel/<id>`, `facebook.com/<page>/videos/<id>`, `fb.watch/<code>`
- These sites switch videos without loading a new page (like YouTube), so the button and the popup list must follow address changes.
- Their players usually play from `blob:` addresses, so finding videos through network requests doesn't work here. Finding them by the page's address is the main way.

**A4. Posts with several videos**
- One Instagram or X post can hold 2 to 10 videos; yt-dlp returns them as a list.
- First version: the quality window shows "Video 1, Video 2, …" and the user picks one.

### Limits
- **Photos aren't included.** yt-dlp downloads videos, not pictures.
- **Facebook and Instagram often send picture and sound separately.** Without FFmpeg (postponed, `omnidm-plan.md` M6 step 3) the user gets two files: `<title>.mp4` without sound and `<title> (sound).m4a`. TikTok and X usually send them together, so there is one file.
- **A video whose picture size yt-dlp doesn't know** (rare: only when no copy of it has a known size) is listed as "Sound only" and saved as `.m4a`, since its size is how a picture is told from sound. When any copy's size is known, the unknown ones are left out.
- **These sites change often**, so downloads can break for a few days until yt-dlp updates. OmniDM already runs `yt-dlp -U` when a read fails.
- These sites' terms restrict downloading, the same situation already accepted for YouTube. The extension isn't published in the Chrome Web Store (`omnidm-plan.md` §16).

### Tests
- Saved yt-dlp answers (JSON) for each site, checking that `manifest_from` reads qualities, sound, headers and cookies correctly.
- Site-list tests in Rust: real links are accepted and look-alike hosts are refused.
- Link-recognition tests in the extension (`*.test.ts`, Vitest), including look-alike addresses.
- A manual test on each real site before release.

### Done when
A public video from each of the four sites downloads from the on-video button and from Add download. Login-only posts work from the browser button, and a post with several videos lets the user pick one.

## Decided

- 2026-11-09 (D1): no FFmpeg for Part A; videos whose picture and sound come apart are saved as two files for now. A copy with both in one file is preferred whenever the site offers one.
- 2026-11-09: torrent download (Part B) and torrent search (Part C) are dropped. The partly built torrent engine (`omnidm-torrent`, librqbit, database schema 8) was deleted before it was ever committed.
