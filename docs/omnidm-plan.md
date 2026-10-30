# OmniDM: Build Plan

> OmniDM is an original, open download manager for Windows, with a companion browser extension.
> This plan is the working spec. The design rationale comes from §10 of `docs/idm-analysis.md`.
> Implementation follows this plan and §10 only, not the descriptive sections of that analysis.

| Item | Decision |
|---|---|
| Product name | OmniDM (binaries and crates: `omnidm-*`) |
| Stack | **Rust** engine + **Tauri 2** desktop app (React + TypeScript + Tailwind CSS UI) + **TypeScript** extension built with **WXT** (React + Tailwind CSS) |
| Licence | **GPL-3.0-or-later**. OmniDM and every derived version stay free and open |
| Repository owner | GitHub [`FahimAnsawry`](https://github.com/FahimAnsawry) |
| First platform | Windows 10/11, x64 and arm64. The core stays cross-platform so macOS/Linux can follow |
| Browsers | Chrome, Edge, Brave, Opera, Vivaldi (one Chromium build) and Firefox |
| Status | Planning. No code yet |
| Plan date | 2026-10-28 |

---

## 1. Scope

### 1.1 Version 1.0 includes

- Segmented HTTP/HTTPS downloads with dynamic splitting, gradual ramp-up, stall handling and safe resume
- The Windows proxy setting (fixed proxy, bypass list, PAC script, auto-detect); a hand-set proxy and SOCKS5 are not in v1 (decided 2026-10-28). Basic and Digest auth, cookies, custom headers, POST
- ~~Global and per-download speed limits~~ (dropped 2026-10-28, see §5.7)
- Queues, scheduler, categories, post-queue actions
- Browser extension: download capture with safe handoff, context menus, media detection, popup, options
- HLS and DASH downloads (FFmpeg remuxing postponed; see M6 step 3)
- Desktop UI, tray icon, notifications, CLI
- Per-user installer, auto-update, extension store listings

### 1.2 Planned for later (v1.x)

- FTP/FTPS (`suppaftp`)
- NTLM/Negotiate auth (see §15, risk R1)
- HTTP/3
- macOS and Linux builds
- Site grabber (spider)
- Import from other download managers

### 1.3 Never

- DRM circumvention, or site-specific signature/cipher solving for streaming sites
- Kernel drivers or network interception
- Reading browser cookie databases or decrypting browser key stores
- An unauthenticated localhost server
- Telemetry without opt-in

---

## 2. Ground rules

1. **Originality.** Don't copy IDM's code, protocol, data formats, wording, icons, layout or identifiers. OmniDM's protocol, schema and UI are designed from scratch.
2. **Never lose a download.** If OmniDM can't take over a browser download, the browser keeps it.
3. **Untrusted input.** Everything from the extension, the pipe, the CLI or a server is untrusted:
   - Accept only `http`, `https` (and later `ftp`) URLs.
   - Keep save paths inside allowed roots.
   - Sanitise file names.
   - Never run downloaded files automatically.
4. **No elevation.** OmniDM runs as the normal user. It installs per user and writes only to user-writable locations.
5. **Crash safety.** A crash or power loss at any moment must leave a resumable download, never a corrupt "complete" file.
6. **Tests first** for the engine. Every resume, error and split rule gets an automated test against `testserver/` before it's considered done.

---

## 3. Identifiers (decided 2026-10-28)

All identifiers derive from the GitHub account `FahimAnsawry`, written in lowercase. They must not change after the first public release.

| Identifier | Value | Notes |
|---|---|---|
| Tauri app identifier | `io.github.fahimansawry.omnidm` | Unique reverse-DNS name based on an account the project owns |
| Native messaging host | `io.github.fahimansawry.omnidm.host` | Lowercase letters, digits, `_` and `.` only |
| Firefox add-on ID | `omnidm@fahimansawry.github.io` | Fixed forever once published |
| Repository | `github.com/FahimAnsawry/OmniDm` | Created in M0 |
| Chromium extension ID | Fixed with the manifest `key` during development | The store assigns the published ID; both go in the host's `allowed_origins` |
| Data folder | `%APPDATA%\OmniDm\` | Portable mode: next to the exe when a `portable` marker file exists |
| Named pipe | `\\.\pipe\OmniDm.<SessionId>` | ACL restricted to the current user (§8.1) |
| Temp file suffix | `.omnidm-part` | Sits next to the final file |

---

## 4. Architecture

### 4.1 Processes

```
 Browser + OmniDM extension (WXT, MV3)
     │  native messaging: 4-byte length + JSON (stdio)
     ▼
 omnidm-host.exe            (tiny relay; starts the app if it isn't running)
     │  named pipe \\.\pipe\OmniDm.<SessionId>, same framing
     ▼
 OmniDm.exe  (Tauri app)
   ├─ React UI (webview)  ◄── Tauri commands / events ──►  Rust backend
   └─ Rust backend: omnidm-core, -net, -store, -queue, -media, -ipc
           │
           ├─ %APPDATA%\OmniDm\omnidm.db   (SQLite, WAL)
           ├─ <save dir>\file.ext.omnidm-part   → renamed on completion
           └─ ffmpeg.exe (remux only)

 omnidm.exe (CLI) ── same named pipe ──► OmniDm.exe
```

### 4.2 Repository layout

```
OmniDM/
  Cargo.toml                 workspace
  crates/
    omnidm-core/             engine: Download, Segment, Scheduler (split/ramp/stall), ResumeValidator
    omnidm-net/              HTTP client wrapper: Windows proxy/PAC, auth, cookies, redirects, file naming
    omnidm-store/            SQLite schema, migrations, repositories, settings
    omnidm-queue/            queues, schedules, categories, power management
    omnidm-media/            HLS and DASH parsing, AES-128 decryption, FFmpeg adapter
    omnidm-ipc/              protocol types (serde), framing, pipe server/client
    omnidm-cli/              `omnidm` command-line client
    omnidm-host/             native messaging host
    omnidm-testserver/       axum server with fault injection (dev only)
  app/
    src-tauri/               Tauri 2 shell: wires crates to commands/events, tray, updater
    src/                     React + TypeScript UI (Vite)
  extension/                 WXT project (Chromium + Firefox builds)
  protocol/                  JSON Schema of the IPC protocol; generates Rust + TS types
  docs/
  .github/workflows/         CI
```

### 4.3 Main libraries (latest stable at M0, pinned by lockfiles)

| Area | Choice | Notes |
|---|---|---|
| Async runtime | `tokio` | |
| HTTP | `reqwest` (`rustls` + platform verifier, `gzip`, `brotli`, `deflate`) | Uses the Windows root store so enterprise CAs work. **Segments use HTTP/1.1** (§5.6) |
| Digest auth | `digest_auth` | |
| PAC | Windows' own engine (`WinHttpGetProxyForUrl`) | No JavaScript engine is bundled; scripts run the way every other Windows app runs them |
| Cookies | `cookie_store` | Per-download jar |
| SQLite | `rusqlite` (bundled) | WAL mode, `PRAGMA user_version` migrations |
| Credentials | `keyring` (Windows Credential Manager) | The database stores only references |
| Windows APIs | `windows` crate | Pipe ACLs, power requests, wake timers, Zone.Identifier |
| Logging | `tracing` + `tracing-appender` | Rolling logs in `%APPDATA%\OmniDm\logs` |
| HLS | `m3u8-rs` | |
| DASH | `dash-mpd` (parser only) | Downloads go through OmniDM's own engine |
| AES | `aes` + `cbc` | HLS AES-128 only |
| File types | `mime_guess`, `infer` | Extension from MIME; sniff HTML error pages |
| Tests | `proptest`, `axum` (test server), Playwright, `web-ext` | |
| UI | React, TypeScript, Vite, **Tailwind CSS v4** (`@tailwindcss/vite`) | Typed commands through `tauri-specta` or hand-written types |
| Extension | WXT + TypeScript + React + **Tailwind CSS v4** | Firefox must be built with `--mv3` because WXT defaults Firefox to MV2 |

---

## 5. Download engine (`omnidm-core`)

### 5.1 Data model

```text
Download {
  id, url, final_url, method, body?, referrer?, user_agent?, headers{}, cookie_ref?,
  save_dir, file_name, total_size?, etag?, last_modified?, resumable,
  status: Queued|Probing|Running|Paused|Completed|Failed|Cancelled,
  queue_id?, category_id?, speed_limit?, max_connections?, created_at, completed_at?, error?
}
Segment { download_id, idx, start, end /*inclusive*/, written /*bytes from start*/ }
```

Invariant: the segments of a download are ordered, don't overlap, and cover `[0, total_size)` exactly. The code checks this invariant and proptest exercises it.

### 5.2 Lifecycle

1. **Probe.** Send `GET` with `Range: bytes=0-` on connection 0. This avoids a separate `HEAD` round trip, and data starts flowing immediately.
   - `206` + `Content-Range: bytes 0-N/T`: resumable, size `T`.
   - `200` + `Content-Length`: size known, not resumable. Single stream, and resume restarts from zero.
   - `200` without a length (chunked): single stream, size unknown.
   - Record the final URL, `ETag`, `Last-Modified`, `Content-Type` and `Content-Disposition`. Decide the file name (§5.8).
2. **Allocate.** Check free space, then create `<name>.omnidm-part`, mark it **sparse** (`FSCTL_SET_SPARSE` on Windows), and set its length to `T` with `File::set_len`.
   - Without the sparse flag, a write far into the file makes Windows zero-fill everything before it first, which the crash test showed freezes big downloads.
   - File systems without sparse support (FAT32) still work, just slower.
   - Use `\\?\` long paths when needed.
3. **Run.** Connection 0 keeps streaming. The ramp-up controller adds connections (§5.3).
4. **Checkpoint** periodically (§5.5).
5. **Complete.**
   - Verify `written == T` for every segment and that the file length is `T`.
   - Verify an optional user-supplied checksum (SHA-256/SHA-1/MD5).
   - Flush, then rename atomically to the final name. On a name conflict, apply the policy (§5.8).
   - Write a `Zone.Identifier` stream (`ZoneId=3`, `ReferrerUrl`, `HostUrl`) as browsers do, so Windows SmartScreen and Office protections still apply.

### 5.3 Ramp-up and splitting

**Controller.** One controller tick per download every **2 s**:

```
target = min(current * 2, max_connections(download, host), HARD_CAP=32)
skip the tick if:
  - a speed limit applies and current speed >= 85% of it (kept in the ramp controller as a hook; no limit is ever set while §5.7 is dropped)
  - the host recently answered 429/503 (back-off window)
  - the largest splittable remainder < 2 * min_segment
for each connection to add:
  donor     = segment with the largest effective remainder
  effective = (donor.end - donor.pos + 1) - donor.speed * 1s      // don't steal what the donor fetches in the next second
  new_start = align_up(donor.end - effective / 2, 64 KiB)
  atomically shrink donor.end to new_start - 1; if donor.pos already passed new_start, abort this split
  new segment [new_start, old_end] on a new connection
```

**Defaults** (all settings, all documented):

| Setting | Default |
|---|---|
| `max_connections` | 8 (1–32), with per-host overrides |
| `min_segment` | 1 MiB |
| Ramp mode | "doubling" or "gentle" (1→2→3→4→6→8) |

**Donor shrink.** Each segment worker reads its `end` from an `AtomicU64` before every write. It writes only up to `end`, then either asks for more work or finishes.

**Reuse.** When a segment finishes, its worker immediately asks the controller for a new split, so the warm keep-alive connection is reused.

### 5.4 Stall handling (hedging)

A segment is *stalled* when its speed stays below 25 % of the median segment speed for 8 s, or it receives no bytes for 15 s.
- **Large remainder** (≥ 2 × `min_segment`): split it (§5.3) so a new connection takes the tail half.
- **Small remainder:** start a **duplicate** request for the remaining range. (Built in M2 as: below 25 % of the download's fastest connection for `hedge_after`, default 8 s.) The duplicate buffers in memory, which is bounded because the remainder is under 2 × `min_segment`. The first to finish commits; the other is cancelled. At most one duplicate per segment.

### 5.5 Disk writes and checkpoints

- Each segment has a write buffer (default 1 MiB). It is written when full **or** after `flush_interval` (default 1 s), so slow connections still save progress.
  - Writes are positional (`FileExt::seek_write`) on a shared handle.
  - M1 writes under the segment-map lock. Moving disk writes out of the lock, to a blocking-pool writer, is a later optimisation.
- **Checkpoint** every 2 s or 16 MiB, whichever comes first:
  1. Flush all buffers.
  2. `FlushFileBuffers`.
  3. Commit every segment's `written` in **one SQLite transaction**.

  The persisted `written` value therefore never runs ahead of what's on disk. After a crash, segments roll back to the last checkpoint, which is always safe.
- Pause, quit and suspend force an immediate checkpoint.

### 5.6 HTTP specifics

- **Multiple connections need HTTP/1.1.** With HTTP/2, `reqwest` would multiplex all segments over one TCP connection and defeat the purpose. Segment clients use `http1_only()` with a pool sized to `max_connections`. HTTP/2 is allowed for single-stream downloads.
- **Range requests:** `Range: bytes=start-end` per segment, plus **`If-Range: <strong ETag or Last-Modified>`** on every ranged request after the probe.
- **Response checks per segment:**
  - `206` with a `Content-Range` that matches the requested start (and end, when bounded) is accepted.
  - `206` with a mismatched range is a protocol error: retry once, then fall back to fewer connections.
  - `200` on a ranged request means the file changed or ranges aren't supported: stop the other segments, then **restart from zero** or ask the user (setting).
  - `416`: re-probe; if the size changed, treat it as a changed file.
- **Errors:**

  | Response | Action |
  |---|---|
  | Network error, timeout, 5xx | Retry with exponential back-off and jitter (1 s → 60 s, default 10 tries per segment) |
  | `429` / `503` | Honour `Retry-After`; from then on use no more connections than the server was serving when it refused; retry |
  | `401` / `407` | Ask for credentials (or use saved ones), retry once. Until M5, a `401` fails with "this site needs a username and password"; a `407` (the proxy wants a password) fails with a clear message |
  | `403`, `404`, `410` on the probe | Fail with a clear message. For a browser capture, report back so the extension resumes the browser's copy (§9.3) |
  | `403` mid-download (expired token) | Pause with "link expired, refresh address". Ask the extension for fresh cookies or a new URL |
- **Content sniffing:** if the first bytes are HTML while a binary was expected, stop and report "server returned a web page instead of the file".

### 5.7 Speed limiting

**Dropped (decided 2026-10-28):** not needed. OmniDM has no speed limit, so the token bucket, `--limit` and the limit settings in the UI are not built. `EngineConfig.speed_limit` stays as an unused hook (always `None`) in case it comes back later.

### 5.8 File naming

Use the first of these that exists:
1. a user-chosen name
2. `Content-Disposition` `filename*` (RFC 5987/6266) or `filename`
3. the last path segment of the final URL, percent-decoded
4. `download`

Then:
- Add or fix the extension from the MIME type when there is none.
- Sanitise:
  - strip `<>:"/\|?*` and control characters
  - avoid reserved names (`CON`, `NUL`, `COM1`…)
  - trim trailing dots and spaces
  - limit the length
- Conflict policy setting: *rename* `name (1).ext` (default), *overwrite after confirmation*, or *ask*.

---

## 6. Network layer (`omnidm-net`)

| Area | Plan |
|---|---|
| Client | One `reqwest::Client` per (proxy config, TLS options) profile, cached. Segment clients are HTTP/1.1 only (§5.6) |
| Redirects | Manual, up to 10 hops, chain recorded. A cross-origin redirect drops `Authorization` and filters cookies by domain |
| Cookies | Captured cookies go into a per-download jar. `Set-Cookie` from the server is merged. Partitioned (CHIPS) cookies are included when the extension provides them |
| Headers | Captured browser headers are replayed except headers the engine controls (`Host`, `Range`, `If-*`, `Connection`, `Content-Length`, `Accept-Encoding`, `Proxy-*`, `Upgrade`) |
| Compression | Request `Accept-Encoding: identity` for ranged segments, so byte offsets match the file. Single-stream downloads may accept gzip/br |
| Auth | Basic, Digest (M5, with the login dialog). Credentials per site in Windows Credential Manager. NTLM/Negotiate in v1.x (§15, R1) |
| Proxy | Follows the Windows setting only (`WinHttpGetIEProxyConfigForCurrentUser`): a PAC script or auto-detect first, falling back to the fixed proxy, as Windows does. Fixed proxies may be one address or `http=…;https=…`; FTP/SOCKS entries are ignored. The Windows bypass list applies (`<local>`, wildcards), and this computer (`localhost`, loopback) is always reached directly. PAC answers are cached per site. reqwest's own system-proxy support is switched off so this plan alone decides. A hand-set proxy and SOCKS5 are not in v1 (decided 2026-10-28) |
| TLS | `rustls` with the platform verifier. "Ignore certificate errors" is a per-download, clearly marked option, off by default |
| IPv6 | Happy Eyeballs through the connector |

---

## 7. Persistence (`omnidm-store`)

A single SQLite database `%APPDATA%\OmniDm\omnidm.db` in WAL mode, with migrations through `PRAGMA user_version`.

```sql
downloads   (id INTEGER PK, url, final_url, method, body BLOB, referrer, user_agent, headers_json,
             cookie_ref, save_dir, file_name, total_size, etag, last_modified, resumable,
             status, queue_id, category_id, speed_limit, max_connections, checksum_algo, checksum,
             created_at, started_at, completed_at, error, source /*cli|extension|ui|clipboard*/, private INTEGER)
segments    (download_id, idx, start, end, written, PRIMARY KEY (download_id, idx))
queues      (id, name, max_concurrent, schedule_json, on_complete /*none|sleep|hibernate|shutdown|quit*/, speed_limit)
queue_items (queue_id, download_id, position)
categories  (id, name, position, extensions, mime_types, host_patterns, save_dir, remember_last_dir)
site_rules  (host_pattern, max_connections, capture /*default|always|never*/, credential_ref, user_agent)
settings    (key PRIMARY KEY, value_json)
history     (download_id, event, at, detail)   -- optional, for the details pane
```

- **Cookies:** encrypted with DPAPI (current user) in a `secrets` table, deleted when the download completes. On resume, OmniDM first asks the extension for fresh cookies when one is connected.
- **Private-window downloads:** no cookies persisted, removed from history on completion (setting).
- **Import/export:** a documented JSON format (all fields) and plain URL lists (`.txt`, one per line).

---

## 8. IPC (`omnidm-ipc`) and protocol

### 8.1 Transport

- Named pipe `\\.\pipe\OmniDm.<SessionId>`, created with `FILE_FLAG_FIRST_PIPE_INSTANCE` and a DACL that allows only the current user's SID.
- Clients verify the server with `GetNamedPipeServerProcessId`: its image path must be OmniDM's install path.
- Framing matches native messaging: **4-byte little-endian length + UTF-8 JSON**. The host relays bytes without re-encoding.
- The pipe doubles as the single-instance lock. Launching `OmniDm.exe <url>` again forwards to the running instance.

### 8.2 Messages

The envelope is `{"v": 1, "id": "<uuid>", "type": "<name>", "payload": {…}}`. Replies carry `"re": "<id>"`. The schema lives in `protocol/` and generates both Rust (serde) and TypeScript types, so the extension, host and app can't drift apart.

| Direction | Type | Payload |
|---|---|---|
| ext → app | `hello` | extension version, browser name and version, protocol versions supported |
| app → ext | `helloAck` | app version, chosen protocol version, capture settings |
| ext → app | `capture` | `url`, `finalUrl?`, `method`, `body?` (base64), `referrer?`, `pageUrl?`, `headers{}`, `cookies[]`, `suggestedName?`, `mime?`, `size?`, `private`, `browserDownloadId` |
| app → ext | `captureResult` | `accepted` \| `rejected` + `reason` (`excluded`, `tooSmall`, `probeFailed:<status>`, `userCancelled`) |
| ext → app | `captureMany` | list of links (from "Download all/selected links") |
| ext → app | `media` | detected stream: `kind` (`hls`, `dash`, `file`), URL, page URL, headers, title |
| app → ext | `cookiesRequest` | `url`: needed when resuming after a restart |
| ext → app | `cookiesResponse` | `cookies[]` |
| app → ext | `settingsChanged` | capture rules, exclusions, minimum size |
| either | `ping` / `pong` | keep-alive every 20 s |
| either | `error` | `code`, `message` |

**Limits:**
- Messages from the host to the extension stay under 1 MB (Chrome's limit). Large lists are paged.
- Messages from the extension are capped at 4 MB by the app.

---

## 9. Browser extension (`extension/`)

### 9.1 Builds

| Build | Command | Notes |
|---|---|---|
| Chromium (Chrome, Edge, Brave, Opera, Vivaldi) | `wxt build` / `wxt zip` | MV3 service worker |
| Firefox | `wxt build -b firefox --mv3` / `wxt zip -b firefox --mv3` | MV3 with event-page background. `browser_specific_settings.gecko.id` set. Minimum Firefox 128 (decided 2026-10-28) |

### 9.2 Permissions

`downloads`, `nativeMessaging`, `contextMenus`, `cookies`, `storage`, `webRequest`, `activeTab`, `scripting`, host permission `<all_urls>`. `scripting` is used only on a user gesture (the context menu) to collect links from the current page; there are no declared content scripts. No `management`, `proxy` or `tabs` in v1. Every extra permission needs a written reason in this plan before it's added.

### 9.3 Capture flow (safe handoff)

```
downloads.onCreated(item)
  ├─ capture disabled, private window not allowed, blob:/data: URL, excluded host or type,
  │  size known and < minimum   → leave it to the browser
  └─ otherwise
       downloads.pause(item.id)
       collect: request headers (from the webRequest cache, §9.4), cookies (cookies.getAll for the URL,
                partitioned too), referrer, page URL, POST body if seen
       send `capture` → wait for `captureResult` (timeout 10 s)
         ├─ accepted → downloads.cancel(item.id) + downloads.erase({id})
         └─ rejected / timeout / host missing → downloads.resume(item.id)
                                                 (+ "Install OmniDM" notice if the host is missing)
```

The app sends `accepted` only after its probe succeeds, so a link that works only inside the browser stays in the browser.

### 9.4 Request header cache

- `webRequest.onSendHeaders` (Chromium: `extraHeaders` so `Cookie`/`Referer` are visible) and `onBeforeRequest` (`requestBody` for POST) fill a small cache keyed by URL, kept for 30 s and 200 entries at most.
- `onCreated` looks up the cache by the download's URL or final URL.
- Nothing is persisted, and nothing from the cache leaves the browser except inside a `capture` message.

### 9.5 Other features

| Feature | Details |
|---|---|
| Context menu | "Download with OmniDM" (links, media), "Download all links", "Download selected links" (the last two inject a one-off link collector with `scripting.executeScript` when the user clicks the menu item) |
| Media detection | `webRequest.onHeadersReceived` for `application/vnd.apple.mpegurl`, `application/x-mpegURL`, `application/dash+xml`, `video/*`, `audio/*`, plus `.m3u8`/`.mpd` URLs. Per-tab list, badge count. Never on DRM-protected streams (an `EXT-X-KEY` method other than `AES-128`, or a DASH `ContentProtection` element), which are shown as "protected, can't download" |
| Popup | Connection state (connected / app not running / not installed), detected media for the tab, capture on/off, "never capture on this site" |
| Options | Capture file types, minimum size, excluded sites, private-window capture, synced with the app through `settingsChanged` |
| Bypass | Turning capture off in the popup, or the per-site rule. An Alt-click bypass needs a content script and is deferred to v1.x |

### 9.6 Extension tests

- Unit tests (Vitest) for rule matching and the header cache.
- End-to-end: Playwright launches Chromium with the unpacked extension and a stub host. It covers plain links, redirects, POST forms, cookie-protected files, expiring links (must fall back to the browser) and media pages from `omnidm-testserver`.
- Firefox: `web-ext run` smoke test.

---

## 10. Native messaging host (`omnidm-host`)

- A tiny Rust exe with no async runtime. It relays stdin ⇄ pipe in two threads and exits when either side closes.
- **App not running:** start `OmniDm.exe --background` from its own directory, then retry connecting to the pipe for up to 10 s.
- Checks that the pipe server is OmniDM (§8.1). Never interprets messages, except to cap their size.
- **Registration** (done by the installer, removed by the uninstaller, both under HKCU):
  - Chrome: `Software\Google\Chrome\NativeMessagingHosts\<host-name>`
  - Edge: `Software\Microsoft\Edge\NativeMessagingHosts\<host-name>`
  - Chromium-based others read Chrome's key. Opera and Vivaldi are tested in M3.
  - Firefox: `Software\Mozilla\NativeMessagingHosts\<host-name>`
  - Manifests: `host-chromium.json` (`allowed_origins` = store ID + development ID) and `host-firefox.json` (`allowed_extensions` = gecko ID).
- The app's settings show a "Repair browser integration" button that rewrites the registration.

---

## 11. Desktop app (`app/`)

### 11.1 Screens

| Screen | Contents |
|---|---|
| Main window | Toolbar (Add, Resume, Pause, Delete, Queue, Settings). Sidebar (All, Downloading, Completed, Failed, categories, queues). List: name, size, progress, speed, time left, status, added |
| Add download | URL (pre-filled from the clipboard), file name, folder (category default), connections, checksum, "start now" or "add to queue" |
| Capture prompt | Shown for captured downloads when "ask before downloading" is on: name, size, folder, category, queue |
| Details | Segment map (bar showing each segment's progress), connections with per-connection speed, headers, redirect chain, log |
| Queues & scheduler | Queue list and order, concurrency, days and times, post-queue action |
| Settings | General, Connections, Categories, Sites, Browser integration, Notifications, Advanced |
| Tray | Show/hide, pause all, resume all, quit |

### 11.2 Backend ↔ UI

- Tauri commands for actions. Events for progress, batched at 5 Hz per download, so a large list stays smooth.
- Notifications and autostart through the official Tauri plugins.
- Keep-awake while downloading through `PowerCreateRequest`/`PowerSetRequest` (system required). Wake timers for schedules through `CreateWaitableTimerExW` with wake enabled (setting).
- The clipboard monitor (setting, off by default) offers a download when a matching URL is copied.

---

## 12. Media (`omnidm-media`)

- **HLS.**
  - Parse the master playlist and let the user pick a variant; parse audio/subtitle renditions.
  - Media playlists: segments, `EXT-X-MAP`, `BYTERANGE`, `DISCONTINUITY`, `MEDIA-SEQUENCE`. Live playlists are out of scope for v1.
  - Segments download in parallel through the engine (one segment = one small download, connection-limited per host).
  - `AES-128` is decrypted. Any other `METHOD` stops with "protected stream".
- **DASH.**
  - Parse the MPD and pick video and audio representations.
  - `SegmentTemplate` (number/time), `SegmentList`, `SegmentBase` + `sidx`.
  - `ContentProtection` → "protected stream".
- **Remux.**
  - `ffmpeg -c copy`: TS → MP4, video + audio → MP4/MKV, with progress parsed from `-progress pipe:1`.
  - FFmpeg is found from settings, the bundled copy or `PATH` (distribution decided in §16).
- **Output:** one final file. Intermediate files go in a `.omnidm-media` folder next to it and are deleted on success.

---

## 13. Testing and CI

| Layer | What |
|---|---|
| Unit | Split math, ramp controller, filename rules, `Content-Disposition` parsing, header filtering, protocol (de)serialisation |
| Property | proptest: after any sequence of splits, completions, stalls and checkpoints, segments still cover `[0, size)` exactly with no overlap, and `written ≤ end − start + 1` |
| Integration | `omnidm-testserver` (axum) with switches: ranges on/off, ignore range (200), change ETag mid-download, throttle a range, drop the connection at byte N, 429 + `Retry-After`, 503, redirect chains, auth (Basic/Digest), cookies required, `Content-Disposition` variants, chunked/unknown size, HTML error page, HLS/DASH fixtures (including AES-128) |
| Crash | Start a large download, kill the process at random points (20 runs), resume, compare SHA-256 |
| Extension | §9.6 |
| UI | Component tests (Vitest), plus a Playwright smoke test of the built app's webview through Tauri's WebDriver support |
| Manual | Real sites: large Linux ISOs, a GitHub release asset (redirect to a signed CDN URL), a site behind login, a PAC script on a company network |

**CI** (GitHub Actions, `windows-latest`):
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
- Extension and UI lint, type-check and tests.
- Build the app and both extension zips as artifacts on every PR.

---

## 14. Milestones

Every milestone ends with its "done when" check passing in CI or by a scripted manual test.

### M0: Setup
- [x] Install rustup (stable), Visual Studio Build Tools (C++ workload; 2026 edition installed), Tauri prerequisites (WebView2 is preinstalled on Windows 10/11), npm.
- [x] `git init`, workspace skeleton (§4.2), CI, `README`, `LICENSE` (GPL-3.0 full text), `CONTRIBUTING`. Every source file starts with `SPDX-License-Identifier: GPL-3.0-or-later`.
- [x] Tailwind CSS v4 in the app and the extension.
- [x] Identifiers decided (§3).
- [ ] Check that the name "OmniDM" is free: GitHub, Chrome Web Store, Edge Add-ons, AMO, a trademark search.
- [x] Rename "FreeDM" to "OmniDM" in `docs/`.
- [ ] Rename the folder `F:\FreeDM` → `F:\OmniDm` (needs Claude Code closed).
- [ ] Create `github.com/FahimAnsawry/OmniDm`, push, and see CI pass.
- **Done when:** `cargo test` and the extension build both pass in CI on an empty skeleton. Locally verified 2026-10-28: fmt, clippy, tests, app UI build, both extension builds.

### M1: Core engine + CLI ✅ (2026-10-28)
- [x] `omnidm-testserver` with the M1 fault switches: ranges off, ETag/size change, throttle, drop, stall, 429/503 + `Retry-After`, unknown size, HTML page, redirects. Cookies and `Content-Disposition` variants move to M2, auth to M5.
- [x] Probe, preallocation, segment workers, ramp controller, splitting, positional writes, checkpoints.
- [x] `If-Range` resume, `200`/`416` handling, retry/back-off, restarting stalled connections.
- [x] SQLite store with migrations; downloads and segments repositories.
- [x] CLI: `omnidm get <url> [-c N] [-o file] [-d dir] [-H header]` and `omnidm list`, running the engine in-process (the pipe comes in M3). (`--limit` was planned for M2, then dropped with §5.7.)
- **Done when:** a 4 GB download with 8 connections survives 20 random kills and matches its SHA-256.
  - **Result:** 4.5 GiB (past the 32-bit offset limit), 8 connections, 20 random kills.
  - After every kill, every byte the database counted as saved was verified on disk, and saved progress never went backwards.
  - The final file is byte-identical to the served file.
  - The 128 MiB version runs in CI (`cargo test -p omnidm-cli --test crash -- --ignored`).
- **Found by the crash test and fixed:**
  - Slow connections held data in memory until the 1 MiB buffer filled, so a crash lost it and progress froze. A connection now also writes after `flush_interval` (1 s).
  - Writing far into a full-size part file made Windows zero-fill everything before it, freezing big downloads for seconds and doubling disk writes. The part file is now sparse (§5.2).
- **Moved to M2:**
  - racing a duplicate request on a small stalled remainder (§5.4)
  - halving connections after 429/503
  - a "link expired" pause for a 403 mid-download
  - a free-space check before allocating (needed now that part files are sparse)

### M2: Network features ✅ (2026-10-28)
- [x] Filename rules (§5.8): `Content-Disposition` (`filename*` UTF-8/ISO-8859-1, `filename`, raw UTF-8 bytes), extension from the MIME type when missing or a script's (`download.php`). The engine has a separate `probe()` so the part file gets the real name; `download()` reuses the probe's response.
- [x] Redirects followed by hand (`omnidm-net::redirect`), at most 10 hops. A `303` (and `301`/`302`, as browsers do) turns a `POST` into a `GET`; `307`/`308` keep it. `Authorization` is only sent to the request's own origin. Recording the redirect chain moves to M5 (the Details view is its only reader).
- [x] Cookies: a per-download jar (`cookie_store`) shared by every connection. It sends each URL only its own cookies and adds `Set-Cookie` from every response, redirects included. A captured `Cookie` header goes into the jar, never raw. Cookies are **not saved** to the database yet (the encryption decision is in M3); the CLI takes them with `-b` on every run.
- [x] Custom headers: replayed except those the engine controls (§6), and `Content-*` body headers only with a `POST`.
- [x] POST: `Request.method`/`body`, the final method after redirects in `RemoteInfo`, and every range request repeats the form. Schema version 2 adds `method`, `body` and `final_method` (old databases upgrade in place). CLI: `--data`.
- Found while testing: while a range waited to retry, the ramp-up treated its connection as free and split it, adding load to a struggling server. Ramp-up now pauses while any range waits to retry.
- [x] Proxy: the Windows setting only (`omnidm-net::proxy`): fixed proxy, bypass list, PAC script and auto-detect through WinHTTP. Tested with a real HTTP test proxy and Windows' PAC engine. A hand-set proxy and SOCKS5 are not built (decided 2026-10-28).
- ~~Token-bucket limiter and `--limit`~~: dropped 2026-10-28 (§5.7).
- [x] The items moved from M1:
  - **Free space:** checked before the part file is made; `EngineError::DiskFull`, with no file left behind. Tested with a pretend 1 PiB file.
  - **Expired links:** a `401`/`403`/`410` after the first response lets running connections finish, opens no new ones, then saves and returns `EngineError::LinkExpired`. The CLI leaves the download paused, so the same command continues once the link works again.
  - **Busy servers:** after a `429`/`503` the download never uses more connections than the server was serving at that moment. That is the number it just proved it accepts, and it beats a blind halving. Before this, a busy mirror could fail the download by using up the retries.
  - **Duplicate requests (§5.4):** a connection below a quarter of the download's best speed for `hedge_after` (8 s), on a range too small to split, gets one duplicate. The first to finish wins and the other is cancelled. A crawling tail went from 8 s to 0.7 s in the test, and a hung one no longer waits for the stall timeout.
- "Ignore certificate errors" (§6 TLS) moves to M5: it is a per-download option that must be clearly marked in the UI.
- Site login (Basic/Digest auth) moved to M5 (decided 2026-10-28). Until then a `401` fails with a clear message.
- **Done when:** the integration suite covers every row in §6 except Auth and passes. Downloading through a fixed proxy and through a PAC script works.

### M3: IPC, host, extension ✅ (2026-10-28)
- [x] Protocol types. The Rust types in `omnidm-ipc::protocol` are the source; ts-rs writes the TypeScript copies to `protocol/ts/` when the tests run, and CI fails if they are stale. This replaced the planned JSON Schema plus generator. Pipe server with the ACL and peer checks. The peer check accepts any OmniDM app found next to the host or CLI (`app_paths`), newest first, so an old build left in the folder doesn't lock out the new one.
- [x] `omnidm-host` and registration: `omnidm-host --register` / `--unregister` for now, the installer in M7. The host uses a small tokio runtime, not plain blocking I/O, because one blocking pipe handle can't read and write at the same time.
- [x] Download service (`omnidm-service`): captures are probed before being accepted, a waiting line (3 at a time), pause/resume. Cookies are saved encrypted with DPAPI until the download completes, and on resume the extension is asked for fresh ones first (decided 2026-10-28).
- [x] App: tray icon (Open/Quit), closing the window hides it, `--background` starts hidden, a second start shows the running one. A minimal live download list; the full UI is M5. A plain `cargo build` of the app needs `--features custom-protocol` to show the built UI instead of the dev server.
- [x] WXT extension: capture flow (§9.3), header cache, context menus, popup, options. Chrome/Edge 120+, Firefox 128+ (decided 2026-10-28). Not yet: partitioned (CHIPS) cookies, and the "Install OmniDM" notice when the host is missing (M5, with the capture prompt and notifications).
- [x] CLI talks to the running app over the pipe; `--standalone` (or `OMNIDM_DATA_DIR`) keeps the old in-process mode.
- **Result (Chrome, by hand):** plain, cookie-protected, referrer-protected and form (POST) downloads all landed in OmniDM complete. The works-once link was refused by OmniDM's probe and finished in the browser. With the host unregistered, the browser downloaded normally.
- Not tested live: Edge (skipped, decided 2026-10-28; it uses the same Chromium code path and registry setup as Chrome) and Firefox (built and linted only; not installed on the dev machine).
- Found while testing: IDM's "advanced browser integration" grabs downloads before any extension sees them. It has to be turned off in IDM to test OmniDM on a PC that has both.
- **Done when:** clicking a download in Chrome, Edge and Firefox lands in OmniDM with cookies and referrer. An expiring link falls back to the browser. Uninstalling the host makes the browser download normally.

### M4: Queues, scheduler, categories ✅ (2026-10-29)
- [x] Queues with concurrency and order. Scheduler with days/times and wake timers.
- [x] Post-queue actions with a 60 s cancellable countdown. Keep-awake.
- [x] Category rules (extension, MIME, host → folder).
- Step 1 done (2026-10-29): the rules and storage.
  - `omnidm-queue`: schedules (once, or a time on chosen weekdays, plus an optional stop time) in the PC's time zone through `jiff`. A time skipped by a daylight-saving jump fires right after it; a repeated time fires once. Post-queue actions; category matching by extension, MIME type (`video/*`) or site, first match wins.
  - Schema version 4: `queues` (starts with "Main", 3 at a time, which can't be deleted), `queue_items` (a download is in one queue at most), `categories`, `downloads.category_id`. A category's rule is stored as one JSON column instead of the three columns in §7.
  - The default categories (Compressed, Documents, Music, Pictures, Programs, Video) have no folder of their own, so downloads keep going to the downloads folder until the user gives a category a folder.
- Step 2 (2026-10-29): queues in the service.
  - A started queue runs its unfinished downloads (waiting, paused or failed) from the top, `max_concurrent` at a time (1–16). Each is tried once per run, so a failing or hand-paused download is passed over, not retried in a loop; starting the queue again tries it again.
  - When nothing is left, the run ends with a "queue finished" update carrying how many downloads it tried (0 for an empty queue, so step 4 can skip the after-queue action). Stopping a queue pauses its downloads and never sends it.
  - Downloads outside queues keep the old rules (captures start at once; others share `max_active`). Queue downloads don't count against `max_active`.
  - CLI: `omnidm queue list|create|edit|delete|start|stop|add|remove|move`, and `omnidm get --queue <name>`. Queue names are unique ignoring case.
- Step 3 (2026-10-29): scheduler and power.
  - `omnidm-service::scheduler` looks at the clock at least every 10 s, and at once when a schedule changes. A start or stop that came due since its last look is acted on even late (a PC that slept through 02:00 without a wake timer starts the queue on waking); when both came due, the later wins. Times from before OmniDM started aren't caught up on.
  - Waking: per queue (`--wake`, off by default), a waitable timer set 60 s before the start. Windows only honours it when "Allow wake timers" is on in the power plan.
  - Keep-awake: a "system required" power request while any download runs, so a timer-woken PC doesn't fall back asleep after about 2 minutes. It shows in `powercfg /requests` as "OmniDM is downloading".
  - CLI: `omnidm queue schedule <name> [--at HH:MM [--days …] | --once "YYYY-MM-DD HH:MM"] [--stop HH:MM] [--wake]`; with no options the queue only starts by hand.
  - Found while testing: a download stopped and only then left the "running" list, so a Resume sent the moment it reported "paused" was ignored. It now leaves the list first (a bug since M3).
- Step 4 (2026-10-29): after-queue actions (`omnidm-service::after_queue`).
  - Only a queue that finishes by itself after trying at least one download triggers its action; an empty or stopped queue never does. One action counts down at a time.
  - If other downloads are still running, it waits for them; if one starts during the countdown, the action is called off.
  - 60 s countdown, reported every second. The app brings its window forward once with a banner and a Cancel button; `omnidm queue cancel-action` cancels from the CLI.
  - Sleep and hibernate use `SetSuspendState` with wake events left on (so a later wake timer still works); shutdown uses `ExitWindowsEx` (apps may save; only hung ones are forced). Both first turn on the shutdown privilege. Quit closes OmniDM.
- Step 5 (2026-10-29): categories applied.
  - A browser download is saved in its category's folder (made when first needed); with no folder, or one that can't be made, in the downloads folder. The category is decided after the probe, from the file name, the server's `Content-Type` and the site; for "download all links", from the link's name.
  - CLI downloads keep their `-d` folder; their category is only recorded.
  - CLI: `omnidm category list` and `omnidm category folder <name> [<folder>]`. `omnidm list` shows each download's category. Adding and editing categories is M5 (Settings → Categories).
- **Done when:** a scheduled queue starts from sleep at the set time, runs 3 at a time, and puts the PC to sleep afterwards.
  - **Not run live** (decided 2026-10-29: the live sleep/wake test was skipped). Covered by the automated tests instead: scheduled starts and stops on chosen clock times and on the real clock, at most N downloads at once, the after-queue countdown and each action with a stand-in for Windows, and Windows accepting the wake timer, the keep-awake request and the shutdown privilege. Waking from sleep and the real sleep have not been seen working.
  - Found while checking: the dev PC's power plan has "Allow wake timers" at "Important wake timers only" (plugged in) and off (battery). Windows then ignores OmniDM's wake timer, so `--wake` silently does nothing. M5 should warn about this where a schedule with waking is set.

### M5: Desktop UI
- [ ] Screens from §11.1, tray, notifications, autostart, clipboard monitor.
- [x] Capture prompt when a download is taken from the browser (with a setting to stay silent), and the extension's "Install OmniDM" notice when the host is missing (§9.3).
- [ ] From M4: queue and schedule screens, category editing (add, remove, reorder, rules), and a warning when "wake the PC" is chosen but Windows' "Allow wake timers" is off. The live sleep/wake test skipped in M4 runs here.
- [ ] Settings wired to the store and live-applied.
- [ ] ~~Site login: Basic and Digest auth, a username/password dialog on `401`, saved per site in Windows Credential Manager. The test server gets auth switches.~~ Dropped by the user (2026-10-29), together with "ignore certificate errors": a `401` still fails with "this site needs a username and password", and a bad certificate fails the download (the browser keeps such captures, since the probe fails first).
- **Done when:** every engine feature is reachable from the UI. 500 downloads in the list scroll smoothly while 10 are running.
- Steps (planned 2026-10-29): 1 main window; 2 Add download dialog, capture prompt, notifications; 3 queues and scheduler screen (with the wake-timer warning) — dropped by the user (2026-10-29): queues and schedules stay CLI-only (`omnidm queue …`), plus Start/Stop queue in the toolbar, and there is no wake-timer warning; 4 Settings (categories, autostart, clipboard monitor, live-applied); 5 Details view; 6 site login and "ignore certificate errors" — dropped by the user (2026-10-29); 7 the extension's "Install OmniDM" notice, then the live tests (sleep/wake, Edge).
- Step 1 (2026-10-29): main window.
  - Toolbar (Resume, Pause, Delete; Start/Stop queue when a queue is shown; search). Sidebar: All, Downloading, Paused, Completed, Failed, the queues (a dot when running) and the categories, each with a count. Status bar with the total speed.
  - List: name, size, progress, speed, time left, status, added. Click a header to sort (then reverse, then back to the view's own order: newest first, or the queue's order). Only the rows in view are drawn, so 500 rows scroll smoothly (checked with 500 made-up downloads).
  - Click, Ctrl-click, Shift-click, arrow keys, Ctrl+A, Delete, Enter. Double-click opens a completed file or resumes a stopped one. Right-click: Open, Show in folder, Resume, Pause, Copy link, Delete.
  - Delete asks first; files stay unless "Also delete the files" is ticked. An unfinished download's part file always goes, since nothing could continue it. A running download is stopped first.
  - Tray: Pause all (also stops started queues) and Resume all (every paused or failed download).
  - Progress reaches the window as one event for all downloads, 5 times a second (§11.2).
  - Found while building: a file saved as `name (1).ext` because `name.ext` was taken kept the planned name in the database, so Open or Delete would have hit the other file. The final name is now saved (a bug since M1).
- Step 2 (2026-10-29): Add download, capture prompt, notifications.
  - Capture prompt (on by default): the capture is still probed and accepted at once (the browser cancels its copy), then held with the server's first response kept for 60 s, and a small always-on-top window asks: file name, category (changing it changes the folder), folder with Browse, "Download now", "Later" in a chosen queue, or Cancel. Closing the window cancels. "Download all links" never asks; those were chosen in the extension.
  - After "Download now" the prompt becomes that download's progress window (decided 2026-10-29): bar, downloaded/total, speed with connections, time left, Pause/Resume, Close (the download keeps going). Finished: Open and Show in folder; failed: the error and Retry. It stops being always-on-top, and its title shows the percentage.
  - "Don't ask again" in the prompt, and "Ask before downloading" in the tray menu, turn it off or on. Saved in the new `settings` table (schema version 5), the start of step 4's settings.
  - A held capture shows as "Asking you" in the list. If OmniDM closes first, it is left paused.
  - Add download (toolbar button, Ctrl+N): link (taken from the clipboard when a link was copied), file name, connections, folder (from the link's category until the user picks one), "Download now" or "Later" in a queue. No checksum field: the engine doesn't check checksums yet.
  - Windows notifications when a download completes or fails, unless the list window is in front. A `notify_finished` setting turns them off (its switch comes with step 4). In a development build Windows labels them "PowerShell"; the installed app (M7) shows its own name.
- Step 4 (2026-10-29): Settings (gear button in the toolbar). Every change is saved in the `settings` table and applied at once; the tray's "Ask before downloading" and the Settings page stay in step.
  - General: downloads folder (made when a download first needs it), start with Windows (`tauri-plugin-autostart`, starts in the tray with `--background`), ask before downloading, clipboard monitor, notifications.
  - Connections: downloads at once (1–16; raising it starts waiting downloads) and connections per download (1–32, for downloads that start afterwards).
  - Categories: add, rename, delete (downloads keep their files and lose the category), reorder (the first match wins), and edit extensions, MIME types, sites and folder. Entries are tidied (`.ZIP` → `zip`, `https://Site.com/x` → `site.com`) and checked.
  - Clipboard monitor (off by default): checks the clipboard every second; a single http(s) link whose name has an extension some category lists opens Add download with it. What is on the clipboard when it is turned on isn't offered.
  - Not included from §11.1: "Sites" (comes with site login, step 6), "Browser integration" (the extension's own options page covers it) and "Advanced".
- Step 5 (2026-10-29): Details window (right-click → Details, or double-click / Enter on a running download). One window per download, refreshed twice a second.
  - Map of pieces: the whole file as one bar, each piece in its place and filled as far as it got (done, downloading, waiting), and a table of pieces with each connection's speed.
  - Server: the link, each redirect (status and target), where the file comes from, method, file type, size, resumable, ETag, Last-Modified, referrer and the headers sent (`Authorization`, `Proxy-Authorization` and `Cookie` values hidden).
  - Log: what happened since OmniDM started (started, what the server sent, redirects, each new connection, retries with the reason, a busy server, a refused link, slow pieces asked for twice, cookies, paused, completed or failed). Kept in memory, at most 500 lines per download; not saved.
  - Engine: new `Event::Segments` (every piece with its state and speed, each tick) and `Event::Log`; `RemoteInfo.redirects`. Schema version 6 stores each download's redirects and `Content-Type` (the redirect chain postponed from M2).
  - A piece's speed shows once its connection has written to disk (every second by default).
- Step 7, part 1 (2026-10-29): the extension's "Install OmniDM" notice, with no new permissions.
  - A badge on the toolbar icon: red "!" while the app can't be found (host not registered), amber "!" when the host ran but the app didn't start; its tooltip says why. It clears once connected and doesn't flicker while reconnecting.
  - The popup shows a red "Install the OmniDM app" notice with Get OmniDM, How to set up and Check again.
  - Right after the extension is first installed, if the app isn't found, a setup page opens once (not on updates). It explains the steps, has a note for source builds (`.\omnidm-host.exe --register` in PowerShell), and checks every 3 s until it turns green.
  - "Get OmniDM" points to the GitHub releases page, which has no installer until M7.

### M6: Media
- [x] HLS and DASH parsers, variant picker, AES-128. (FFmpeg adapter postponed, see step 3.)
- [x] Extension media list wired to the app. (Live browser test still to do.)
- **Done when:** HLS (clear and AES-128) and DASH test fixtures download and join into playable files (`.ts`/`.mp4`, plus a separate sound file when the stream has one). Protected streams are refused with a clear message.
- **Steps:** 1. reading HLS and DASH; 2. downloading the pieces; 3. FFmpeg joining (postponed); 4. quality picker in the app; 5. the extension's media list.
- Step 1 (2026-10-29): `omnidm-media` reads both kinds into one model, with no network code.
  - A `Manifest` lists qualities (best first) and separate sound tracks. Each one is either ready to download (a `Track`: an optional header piece, then pieces with byte ranges and AES-128 keys) or, for an HLS master playlist, a media playlist still to fetch.
  - HLS: I-frame and backup variants are skipped. Keys and maps carry over until the next tag, and the IV defaults to the sequence number. A byte range without an offset follows the previous one. A header piece means fragmented MP4. A header that changes partway through is refused for now.
  - DASH: uses `dash-mpd-core`, the parser-only half of `dash-mpd` (0.21 now downloads by itself and pulls in about 30 crates). It handles `SegmentTemplate` with a timeline (including `r="-1"`) or `@duration`, settings inherited from the levels above, `$Number%05d$`-style widths, `SegmentList`, and one-file `SegmentBase` (downloaded whole, so no `sidx` reading). Several periods are refused for now.
  - Refused with a clear message: DRM (`SAMPLE-AES`, non-`identity` key formats, any `ContentProtection`, named Widevine/PlayReady/FairPlay where possible), live streams, non-web addresses, and more than 200,000 pieces.
  - m3u8-rs 6.0.1 drops `EXT-X-KEY:METHOD=NONE` (its IV check is the wrong way round); OmniDM reads it from the unknown tags instead.
  - Subtitles aren't offered yet.
- Step 2 (2026-10-29): downloading the pieces (`Manager::add_media`; nothing calls it yet, that is steps 4–5).
  - The playlist is read before anything is added, so DRM, live streams and dead links are refused with their reason and leave nothing in the list.
  - `omnidm_core::fetch` gets one small file or byte range over a single connection with the engine's rules: redirects by hand, cookies, captured headers, retries, the stall timeout, and the HTML-page check (off for playlists). It continues from bytes already held, and slices the range out itself when a server ignores `Range`. Playlists and keys are capped at 64 MB in memory.
  - Work folder `<name>.omnidm-media/` next to the file, one folder per track: `<n>.part` while a piece downloads, `<n>.piece` once it is complete (and unlocked). A `track-id` fingerprint (piece addresses without their query, plus ranges, FNV-1a) starts a track over if the playlist changed. Pausing keeps complete pieces and half-done ones; resuming reads the playlist again (fresh tokens), skips complete pieces and continues the rest.
  - Pieces download `max_connections` at a time; each AES-128 key is fetched once. 401/403/410 on a piece or key pauses with "link expired", keeping the pieces.
  - The chosen quality (height + bandwidth) and sound track (name + language) are saved as `media_json` (schema 7, with `media_saved` for progress). A resume takes the same ones, or the closest if the playlist changed.
  - Each track's pieces are joined in order into one file. Until FFmpeg (step 3): one track is saved as `<title>.ts|mp4|webm`, and a separate sound track as `<title> (sound).m4a|webm`.
  - The file name comes from the page title, else the playlist's name, or its folder's when that is generic (`master`, `index`, `manifest`…).
- Step 3 (2026-10-29): **postponed by the user**: no FFmpeg for now, maybe later. Videos stay as step 2 saves them: `.ts` when the pieces are TS (plays in VLC and most players), and two files when picture and sound come separately (`<title>.mp4` without sound, `<title> (sound).m4a`). If FFmpeg is added later, it plugs in where `save_tracks` moves the joined tracks: remux TS to MP4 and merge picture + sound with `ffmpeg -c copy`.
- Step 4 (2026-10-29): the quality window in **Add download**.
  - A link whose path ends in `.m3u8`, `.m3u` or `.mpd` is read as a video once typing stops (`look_at_media`): qualities best first (resolution, bitrate, estimated size), the sound track when there is a choice, and the length. For an HLS master playlist the best quality's playlist is also read, for the length.
  - Sizes are bandwidth × length; DASH adds the default sound's bandwidth (HLS already counts it). DRM, live streams and dead links show their reason, and the Download buttons stay off.
  - The file name defaults to the playlist's name (or its folder's when generic); the folder follows the Video category. "Later" puts the video in a queue (`MediaAdd::queue`). The Connections box is hidden for videos: pieces download `max_connections` at a time.
  - A note says when picture and sound will be saved as two files (no FFmpeg).
  - `MediaPicker` is its own component, for step 5 to reuse. Checked against real playlists: Mux's Big Buck Bunny (HLS, 5 qualities), Apple's bipbop sample (HLS, 8 qualities, separate sound groups), Akamai's bbb_30fps (DASH, 10 qualities up to 4K, 159 pieces each).
- Step 5 (2026-10-29): the extension finds videos on pages; no new permissions.
  - `webRequest.onHeadersReceived` (xmlhttprequest, media, other) picks out HLS/DASH playlists by content type, or else by a `.m3u8`/`.mpd` name. Only the page's own requests count; the extension's reads have no tab.
  - Each new playlist is read once (a `fetch` with the browser's cookies, 10 s, at most 5 MB) to tell: an HLS master's qualities and sound playlists (hidden as parts of it, whichever arrives first), DRM (`SAMPLE-AES`, non-`identity` key formats, session keys, `ContentProtection`), live (no `ENDLIST` and not VOD; `type="dynamic"`), and things that aren't video playlists (an internet-radio `.m3u`), which are dropped. A playlist that can't be read is listed as ready; the app says if it can't be downloaded.
  - Per tab, at most 50, cleared when the tab loads another page or closes, and kept in `storage.session` because the background can be stopped.
  - The badge counts the page's downloadable videos (per tab, sky blue); the red/amber "!" notice wins while the app can't be found.
  - The popup lists "Videos on this page": a name (the playlist's, or its folder's when generic), HLS/DASH, or why it can't be downloaded (checking, protected, live), and a Download button.
  - Download sends `media` (kind, URL, page, tab title, cached request headers, cookies, private) to the app, answered `ack` once the window is up. The service holds it as an offer (at most 32; `Update::MediaOffer`) and the app opens a `media-<offer>` window: the quality picker plus name, folder, Cancel / Later (queue) / Download now, which then shows the download's progress, like the capture prompt. Closing the window drops the offer.
- Step 5b (2026-10-29, asked for by the user): a Download button on the video itself.
  - The background puts `video-button.js` (an unlisted script) into the frame that asked for the playlist (`webRequest` gives its `frameId`), only once the playlist was read as downloadable, and only with the "Show a Download button on videos" setting on (default on). So nothing runs on pages without a video, and no new permissions or manifest content scripts are needed.
  - The button (a download icon) shows over the top-right corner of a video at least 240×135 while the pointer is on it, and stays 1.5 s after it leaves. It follows scrolling and resizing, and moves into a full-screen player (not a bare full-screen `<video>`). It is drawn in a closed shadow root; text from the network is only set as text. Its click doesn't reach the page's player.
  - Clicking it opens a menu of the qualities (name and estimated size) of the videos found in that frame (else the whole tab's), grouped by name when there are several. The app reads them (`mediaLook` → `mediaInfo`, with the page's cookies), and the background keeps the answer for 2 minutes.
  - Picking a quality sends `media` with `quality` (height + bandwidth): the app's video window opens showing that quality as one line (not the list again), for the name, folder and Download now. The button is a round icon only ("Download with OmniDM" as its tooltip and label).
- Step 6 (2026-10-29, asked for by the user): YouTube, through yt-dlp. The user won't publish the extension in the Chrome Web Store, whose policy forbids YouTube downloading; YouTube's own terms restrict it too.
  - YouTube plays no standard playlist, and scrambles its format addresses in ways that change often, so OmniDM doesn't unscramble them itself: `omnidm_service::ytdlp` asks yt-dlp (`--dump-single-json --no-playlist --ignore-config`, no console window, 90 s limit, the page's cookies as a `Cookie` header) and downloads the formats with its own engine.
  - yt-dlp is fetched on first use from its latest GitHub release into `<data>/tools/`, checked against the release's `SHA2-256SUMS` (a mismatch is deleted), and kept. When reading a video fails, `yt-dlp -U` runs once and the video is read again. `Manager::set_ytdlp` points elsewhere (tests, or a later "my own copy" setting).
  - Formats become a manifest (`StreamKind::YouTube`, with the video's own title): plain `https` formats without DRM; per height and frame rate the best picture (a format with its sound first, then MP4, then bitrate); per language the best sound (M4A first, the original first). Live and upcoming videos are refused.
  - Each format with a known size is cut into 10 MB byte-range pieces downloaded in parallel (YouTube serves big single requests slowly). Tracks gained an `id` (the format number, part of the resume fingerprint, since addresses change) and their own request `headers` (yt-dlp names them per format). Resuming asks yt-dlp again, since YouTube's addresses expire after a few hours.
  - Above 360p, picture and sound come apart: saved as two files until FFmpeg is added (step 3).
  - The extension knows YouTube watch pages, shorts and `youtu.be` links by address (YouTube changes videos without loading a page), and embedded players (`/embed/` frames, once loaded). They get the button and are listed in the popup. Add download accepts YouTube links too.

### M7: Release
- [ ] Tauri bundler: **NSIS per-user installer** (no admin; it writes the HKCU native-messaging keys). `omnidm-host.exe` shipped through `bundle.externalBin`, and the uninstaller removes the keys.
- [ ] Code signing (OV certificate or Azure Trusted Signing) for the app, host and installer.
- [ ] Auto-update with `tauri-plugin-updater` and a signed update feed.
- [ ] Store listings: Chrome Web Store (one-time US$5 developer fee), Edge Add-ons, Firefox AMO. Privacy policy page (no data leaves the PC).
- [ ] User guide and FAQ.
- **Done when:** a clean Windows VM goes from installer, to extension install from the store, to a captured download, to an auto-update to the next version, with no admin prompt.

**Parallel work:** the extension skeleton (popup, options, WXT builds) and UI mock-ups can start during M1–M2, because they don't depend on the engine.

---

## 15. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | NTLM/Negotiate isn't built into `reqwest` | v1.x: SSPI through the `windows` crate (or the `sspi` crate) on a custom auth step. Until then, report "server requires Windows authentication" and let the browser keep the download |
| R2 | HTTP/2 multiplexing hides multiple connections | Segments are HTTP/1.1 only (§5.6) |
| R3 | Matching a download to its request headers in the extension is fuzzy (no shared request ID) | Key by URL and final URL with a short time window. Fall back to cookies plus referrer only. The safe handoff keeps failures harmless |
| R4 | Links bound to the browser session (one-time tokens, fingerprinted CDNs) fail in OmniDM | The app probes before `accepted`, so the browser keeps the download |
| R5 | Store review of `<all_urls>` + `webRequest` + `nativeMessaging` | Minimal permissions, a clear listing that explains each one, a privacy policy, no remote code |
| R6 | SmartScreen and antivirus warnings for an unsigned new download manager | Code signing from the first public build, submissions to AV vendors, reproducible builds |
| R7 | FFmpeg size and licence notices | FFmpeg runs as a separate program. If bundled, ship its licence and a source link; see §16 |
| R8 | Disk-full or path errors mid-download | Free-space check before preallocation; a write error pauses the download with a clear message, never fails silently |

---

## 16. Open decisions

| Decision | Options | Needed by |
|---|---|---|
| FFmpeg distribution | Bundle a build (+~80 MB; GPL or LGPL builds are both compatible with OmniDM's GPL-3.0) / download on first use / user-provided path. **Postponed 2026-10-29:** no FFmpeg for now; decide if it is added later | Later |
| Persist cookies for resume | **Decided 2026-10-28: DPAPI-encrypted** (current user) in the database until the download completes | ✅ |
| Chrome Web Store | **Decided 2026-10-29: not published there** (the user's choice; it allows OmniDM's YouTube support) | ✅ |
| Minimum browser versions | **Decided 2026-10-28: Chrome/Edge 120+, Firefox 128+** (the ESR line) | ✅ |
| MSI in addition to NSIS | Only if enterprise deployment matters | M7 |
| Optional crash reports | None (proposed) or opt-in | M7 |
