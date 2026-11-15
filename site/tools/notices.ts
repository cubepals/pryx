// SPDX-License-Identifier: FSL-1.1-ALv2
/** Retain upstream licence/notice files with the static artifact. This inventories installed
 * packages only; it neither downloads dependencies nor copies their implementations. */
import { lstatSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { files } from './artifact.ts'
import { site } from './sources.ts'

export function writeNotices(directory: string) {
  const required = new Set(['astro', '@astrojs/starlight', 'pagefind', '@pagefind/default-ui'])
  const notices = new Map<string, string>()
  const visit = (nodeModules: string) => {
    if (lstatSync(nodeModules).isSymbolicLink())
      throw new Error('Licence inventory refuses module-directory links')
    for (const entry of readdirSync(nodeModules).sort()) {
      if (entry.startsWith('.')) continue
      if (lstatSync(join(nodeModules, entry)).isSymbolicLink())
        throw new Error('Licence inventory refuses scope links')
      const paths = entry.startsWith('@')
        ? readdirSync(join(nodeModules, entry)).map((name) => join(nodeModules, entry, name))
        : [join(nodeModules, entry)]
      for (const path of paths) {
        if (lstatSync(path).isSymbolicLink()) throw new Error('Licence inventory refuses package links')
        if (!lstatSync(path).isDirectory()) continue
        const names = readdirSync(path).sort()
        if (!names.includes('package.json')) continue
        const metadata = JSON.parse(readFileSync(join(path, 'package.json'), 'utf8')) as {
          name: string
          version: string
        }
        const texts: string[] = []
        for (const file of names.filter((name) => /^(?:licen[cs]e|copying|notice)(?:[.-]|$)/i.test(name))) {
          const full = join(path, file)
          const stat = lstatSync(full)
          if (stat.isSymbolicLink()) throw new Error('Licence inventory refuses notice links')
          if (stat.isFile()) texts.push(`${file}\n${readFileSync(full, 'utf8')}`)
          else if (stat.isDirectory())
            for (const child of files(full))
              texts.push(`${file}/${child}\n${readFileSync(join(full, child), 'utf8')}`)
        }
        if (texts.length) {
          notices.set(`${metadata.name} ${metadata.version}`, texts.join('\n\n'))
          required.delete(metadata.name)
        }
        if (names.includes('node_modules')) visit(join(path, 'node_modules'))
      }
    }
  }
  visit(join(site, 'node_modules'))
  const pagefind = JSON.parse(readFileSync(join(site, 'node_modules/pagefind/package.json'), 'utf8')) as {
    version: string
  }
  const ui = JSON.parse(
    readFileSync(join(site, 'node_modules/@pagefind/default-ui/package.json'), 'utf8'),
  ) as { version: string }
  if (pagefind.version !== ui.version) throw new Error('Pagefind UI and owner licence versions differ')
  notices.set(
    `@pagefind/default-ui ${ui.version}`,
    readFileSync(join(site, 'node_modules/pagefind/LICENSE/LICENSE'), 'utf8'),
  )
  required.delete('@pagefind/default-ui')
  for (const file of files(join(site, 'notices')))
    notices.set(file, readFileSync(join(site, 'notices', file), 'utf8'))
  if (required.size) throw new Error(`Missing upstream static-client notices: ${[...required].join(', ')}`)
  writeFileSync(
    join(directory, 'THIRD-PARTY-NOTICES.txt'),
    [...notices]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([name, text]) => `${name}\n${text}`)
      .join('\n\n--------------------\n\n'),
  )
  return notices.size
}
