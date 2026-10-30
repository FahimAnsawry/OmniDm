# OmniDM

OmniDM is a free, open-source download manager for Windows, with a companion browser extension for Chrome, Edge, Brave, Opera, Vivaldi and Firefox.

> **Status:** early development (milestone M0, project setup). Nothing is ready to use yet.

## Planned features

- Fast multi-connection downloads that split dynamically and resume safely after a crash or restart
- Speed limits, queues, a scheduler and categories
- Browser integration: downloads are handed to OmniDM, and stay in the browser if OmniDM can't take them
- HLS and DASH video downloads (no DRM circumvention)
- Proxy, PAC and authentication support

The full design is in [`docs/omnidm-plan.md`](docs/omnidm-plan.md).

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | Rust engine and tools: `omnidm-core`, `-net`, `-store`, `-queue`, `-media`, `-ipc`, `-cli`, `-host`, `-testserver` |
| `app/` | Desktop app: Tauri 2 (`src-tauri/`) with a React + TypeScript + Tailwind CSS UI (`src/`) |
| `extension/` | Browser extension built with WXT, React and Tailwind CSS |
| `docs/` | Plan and design documents |

## Building

Requirements: [Rust](https://rustup.rs) (stable), Visual Studio Build Tools with the "Desktop development with C++" workload, and Node.js 20 or newer.

```sh
# Desktop app
cd app
npm install
npm run tauri dev

# Rust workspace (build the app UI first: the Tauri crate embeds app/dist)
cargo test --workspace

# Browser extension
cd extension
npm install
npm run build            # Chromium
npm run build:firefox    # Firefox
```

## Licence

OmniDM is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
