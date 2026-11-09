# Repository Guidelines

## Project Structure & Module Organization

OmniDM is a Windows download manager. `crates/omnidm-*` contains the Rust engine, networking, storage, queues, media, service, IPC, browser host, CLI, and test server. Integration tests live in each crate's `tests/` directory. `app/src/` contains the React/TypeScript desktop UI; `app/src-tauri/` contains its Tauri backend. `extension/` contains the WXT browser extension, with entrypoints and colocated library tests. `installer/` and `scripts/` handle packaging. `docs/` holds architecture, release notes, and README assets.

## Build, Test, and Development Commands

Use Windows, Rust stable (minimum 1.90, edition 2024), Visual Studio C++ Build Tools, and Node.js 22 to match CI. Install npm dependencies separately in `app/` and `extension/` with `npm ci`.

- In `app/`, `npm run tauri dev` starts desktop development; `npm run build` builds the UI. Build it before workspace Rust checks because Tauri embeds `app/dist`.
- At the root, `cargo fmt --all --check` checks formatting; `cargo clippy --workspace --all-targets -- -D warnings` enforces lint checks; `cargo test --workspace` runs Rust tests.
- In `app/`, `npm run typecheck` checks TypeScript.
- In `extension/`, `npm run compile` checks types, `npm test` runs Vitest, and `npm run build` / `npm run build:firefox` build both browser targets.
- At the root, `.\scripts\build-setup.ps1` creates release packages.

## Coding Style & Naming Conventions

Use rustfmt defaults: four-space Rust indentation and `snake_case` functions/modules. Follow existing two-space TypeScript indentation and local quote conventions. Use PascalCase React component files and camelCase utilities. No dedicated JavaScript formatter or linter is configured. Start new source files with `// SPDX-License-Identifier: GPL-3.0-or-later`. Modify Rust protocol definitions in `crates/omnidm-ipc/src/protocol.rs`; tests regenerate `protocol/ts/` bindings.

## Testing Guidelines

Engine changes require regression tests against `omnidm-testserver`, covering resume, errors, and splitting. Rust uses standard/Tokio tests and proptest; extension tests use `*.test.ts` and descriptive behavior names. No numeric coverage threshold is configured. For crash/resume changes, also run `cargo test -p omnidm-cli --test crash -- --ignored --nocapture`.

## Commit & Pull Request Guidelines

History favors short, imperative subjects, such as “Add auto-update and the release package.” Keep commits focused. Describe behavior changes, link relevant issues, report checks performed, and include screenshots for UI changes. Follow `CONTRIBUTING.md`; discuss architecture changes against `docs/omnidm-plan.md`. Contributions must be original or GPL-compatible with attribution.
