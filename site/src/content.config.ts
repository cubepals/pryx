// SPDX-License-Identifier: FSL-1.1-ALv2
/** Load repository-owned pages directly with Astro, then verify the actual content store.
 * Rendering belongs to Starlight; this loader never edits the canonical Markdown files. */

import { defineCollection } from 'astro:content'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { docsSchema } from '@astrojs/starlight/schema'
import { glob } from 'astro/loaders'
import { callerBlocks, root, routeId, site, source, sourceFiles, verifySources } from '../tools/sources.ts'

const canonical = glob({
  base: '..',
  pattern: ['docs/**/*.md', 'sdk/typescript/README.md', 'sdk/python/README.md'],
  generateId: ({ entry }) => routeId(entry),
})
export const collections = {
  docs: defineCollection({
    schema: docsSchema(),
    loader: {
      name: 'pryx-canonical-docs',
      async load(context) {
        sourceFiles()
        await canonical.load({
          ...context,
          store: {
            ...context.store,
            set(entry) {
              const body = source(relative(root, join(site, entry.filePath as string))).body
              if (entry.body !== body.trim() && entry.body !== body)
                throw new Error(`Page differs from canonical source: ${entry.filePath}`)
              return context.store.set({ ...entry, body })
            },
          },
          parseData: ({ id, data, filePath }) =>
            context.parseData({
              id,
              data: { title: source(relative(root, filePath as string)).title, ...data },
              filePath,
            }),
        })
        const pages = verifySources(context.store.values())
        mkdirSync(join(site, '.build'), { recursive: true })
        writeFileSync(join(site, '.build/sources.json'), JSON.stringify(pages, null, 2))
        writeFileSync(join(site, '.build/connect.json'), JSON.stringify(callerBlocks(), null, 2))
      },
    },
  }),
}
