// SPDX-License-Identifier: FSL-1.1-ALv2
/** Canonical documentation inventory, content parity and caller fences. This module never
 * generates a second Markdown tree or reads through a source symlink. */
import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import matter from 'gray-matter'
import { fromMarkdown } from 'mdast-util-from-markdown'

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
export const site = join(root, 'site')
export const digest = (body: string) => createHash('sha256').update(body).digest('hex')
export function routeId(file: string) {
  if (file === 'docs/README.md') return 'reference'
  if (file.startsWith('sdk/')) return file.replace('/README.md', '')
  return file
    .replace(/^docs\//, '')
    .replace(/\.md$/, '')
    .replace(/\/README$/, '/index')
}
export function sourceFiles(directory = root) {
  const files: string[] = []
  const visit = (file: string) => {
    const full = join(directory, file)
    const stat = lstatSync(full)
    if (stat.isSymbolicLink()) throw new Error(`Documentation source is a symlink: ${file}`)
    if (stat.isDirectory()) {
      for (const name of readdirSync(full).sort()) visit(`${file}/${name}`)
    } else if (file.endsWith('.md')) files.push(file)
  }
  visit('docs')
  for (const file of ['sdk', 'sdk/typescript', 'sdk/python']) {
    if (lstatSync(join(directory, file)).isSymbolicLink())
      throw new Error(`Documentation source is a symlink: ${file}`)
  }
  visit('sdk/typescript/README.md')
  visit('sdk/python/README.md')
  return files.sort()
}
export function source(file: string, directory = root) {
  const parsed = matter(readFileSync(join(directory, file), 'utf8'))
  const heading = /^#{1,6}\s+(.+)$/m.exec(parsed.content)?.[1]
  const title =
    parsed.data.title ?? heading ?? file.split('/').at(-1)?.replace(/\.md$/, '').replaceAll('-', ' ')
  if (typeof title !== 'string' || !title.trim()) throw new Error(`Missing page title: ${file}`)
  return { file, id: routeId(file), body: parsed.content, title, data: parsed.data }
}
export function verifySources(
  entries: Array<{ id: string; filePath?: string; body?: string }>,
  directory = root,
  collectionRoot = site,
) {
  const expected = new Map(sourceFiles(directory).map((file) => [file, source(file, directory)]))
  const seen = new Set<string>()
  const routes = new Set<string>()
  const pages = entries.map((entry) => {
    if (!entry.filePath) throw new Error(`Page has no canonical file: ${entry.id}`)
    const file = relative(directory, resolve(collectionRoot, entry.filePath)).replaceAll('\\', '/')
    const canonical = expected.get(file)
    if (!canonical || seen.has(file)) throw new Error(`Unexpected or repeated page: ${file}`)
    if (entry.body !== canonical.body) throw new Error(`Page differs from canonical source: ${file}`)
    if (entry.id !== canonical.id || routes.has(entry.id)) throw new Error(`Page route drift: ${file}`)
    seen.add(file)
    routes.add(entry.id)
    return { file, id: entry.id, digest: digest(canonical.body) }
  })
  for (const file of expected.keys()) if (!seen.has(file)) throw new Error(`Page missing from site: ${file}`)
  return pages.sort((a, b) => a.file.localeCompare(b.file))
}
export function callerBlocks() {
  return sourceFiles()
    .filter((file) => file.startsWith('docs/connect/'))
    .map((file) => {
      const document = fromMarkdown(source(file).body)
      const codes = document.children.filter((node) => node.type === 'code')
      const steps = document.children.filter((node) => node.type === 'list').filter((node) => node.ordered)
      return {
        file,
        id: routeId(file),
        codes: codes.map((node) => ({ language: node.lang, code: node.value })),
        steps: steps.map((list) => list.children.length),
      }
    })
}
