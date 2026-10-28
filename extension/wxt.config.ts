// SPDX-License-Identifier: GPL-3.0-or-later

import { defineConfig } from 'wxt';
import tailwindcss from '@tailwindcss/vite';

// Permissions are added in milestone M3, each one justified in docs/omnidm-plan.md §9.2.
// See https://wxt.dev/api/reference/wxt/interfaces/inlineconfig.html
export default defineConfig({
  modules: ['@wxt-dev/module-react'],
  vite: () => ({
    plugins: [tailwindcss()],
  }),
  manifest: ({ browser }) => ({
    name: 'OmniDm',
    description: 'Send downloads from your browser to the OmniDm download manager.',
    ...(browser === 'firefox' && {
      browser_specific_settings: {
        gecko: { id: 'omnidm@fahimansawry.github.io' },
      },
    }),
  }),
});
