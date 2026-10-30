// SPDX-License-Identifier: GPL-3.0-or-later

import { defineConfig } from 'wxt';
import tailwindcss from '@tailwindcss/vite';

// The public half of the development signing key. It fixes the Chromium extension ID to
// mohldghkdbkpfldhfmfndfcamcailkai, which the native host allows (crates/omnidm-host).
// The store assigns its own ID on publishing (M7).
const DEV_KEY =
  'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAwKBAX/BE0tZEyJ3tkdsjn2y1kKYAKHv0nEWcEay9RR5cBpJKvVXUhYgFCd6nUu8/JGEpsEPtiJsAywG/UOAaa5IdXct63UlYRS2PcRZ/zV7EYTZhsgWPo+qIyIIawhUPLZYyZeNCtbKBDHR3GJ2g5Fmxywy3Hax9I7wCbPrIvf0C2cTKJsohR5z8mm+61ey1B55G1zz5+aFXXZYUNVXOvL/EsNNZMum+jCrRdo1hVpIm4FRC7FPSlP8nYLfaOzqDh33wqVdkIeySxamWXP4NGkBbXOxYxS6ajeu6IYPccz2Zy2EM7tR51aQ2CFRW+lFN2LMHlNleYq7SpyxkhTubvQIDAQAB';

// Each permission is justified in docs/omnidm-plan.md §9.2.
// See https://wxt.dev/api/reference/wxt/interfaces/inlineconfig.html
export default defineConfig({
  modules: ['@wxt-dev/module-react'],
  vite: () => ({
    plugins: [tailwindcss()],
  }),
  manifest: ({ browser }) => ({
    name: 'OmniDM',
    description: 'Send downloads from your browser to the OmniDM download manager.',
    action: {
      default_title: 'OmniDM (click to disable)',
    },
    permissions: [
      'downloads',
      ...(browser === 'firefox' ? [] : ['downloads.ui']),
      'nativeMessaging',
      'contextMenus',
      'cookies',
      'storage',
      'webRequest',
      'activeTab',
      'scripting',
    ],
    host_permissions: ['<all_urls>'],
    ...(browser === 'firefox'
      ? {
          browser_specific_settings: {
            gecko: {
              id: 'omnidm@fahimansawry.github.io',
              strict_min_version: '128.0',
              // Nothing leaves the computer: data only goes to the local OmniDM app.
              data_collection_permissions: { required: ['none'] },
            },
          },
        }
      : { key: DEV_KEY, minimum_chrome_version: '120' }),
  }),
});
