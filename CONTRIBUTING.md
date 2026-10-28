# Contributing to OmniDm

Thanks for helping! A few rules keep the project healthy.

## Ground rules

- **Original work only.** Don't copy code, text, icons, protocols or data formats from other download managers, including closed-source ones. Contributions must be your own work or come from a GPL-3.0-compatible source with attribution.
- **Licence.** By contributing, you agree that your contribution is licensed under GPL-3.0-or-later. Start every new source file with:
  ```
  // SPDX-License-Identifier: GPL-3.0-or-later
  ```
- **Design first.** Bigger changes should match [`docs/omnidm-plan.md`](docs/omnidm-plan.md). If they don't, open an issue to discuss updating the plan.
- **Tests.** Engine changes need tests. Resume, error and splitting behaviour is tested against `crates/omnidm-testserver`.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd app && npm run typecheck && cd ..
cd extension && npm run compile && npm run build && npm run build:firefox
```

CI runs the same checks on Windows.
