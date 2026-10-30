# OmniDM browser extension

Built with [WXT](https://wxt.dev). One TypeScript codebase produces both builds:

```sh
npm install
npm run dev              # Chrome with hot reload
npm run build            # Chromium build (Chrome, Edge, Brave, Opera, Vivaldi) in .output/
npm run build:firefox    # Firefox MV3 build in .output/
npm run zip && npm run zip:firefox   # store packages
```

Firefox scripts pass `--mv3` because WXT targets MV2 for Firefox by default.
See `docs/omnidm-plan.md` §9 for the extension design.
