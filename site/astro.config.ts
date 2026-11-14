// SPDX-License-Identifier: FSL-1.1-ALv2

import starlight from '@astrojs/starlight'
import { defineConfig } from 'astro/config'
import { address, base, version } from './tools/settings.ts'

export default defineConfig({
  site: address.origin,
  base,
  outDir: `./dist/v${version}`,
  trailingSlash: 'always',
  build: { inlineStylesheets: 'always' },
  integrations: [
    starlight({
      title: 'pryx',
      description: 'Sandboxes on Linux hosts you own.',
      customCss: ['./src/style.css'],
      sidebar: [
        {
          label: 'Host pryx',
          items: [{ label: 'Overview', link: '/hosting/' }, { autogenerate: { directory: 'host' } }],
        },
        {
          label: 'Connect',
          items: [
            { label: 'Choose a caller', link: '/connect/' },
            { autogenerate: { directory: 'connect' } },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'All documentation', link: '/reference/' },
            { label: 'Tiers', link: '/tiers/' },
            { label: 'API', link: '/api/' },
            { label: 'MCP', link: '/mcp/' },
            { label: 'TypeScript SDK', link: '/sdk/typescript/' },
            { label: 'Python SDK', link: '/sdk/python/' },
          ],
        },
      ],
    }),
  ],
})
