// SPDX-License-Identifier: FSL-1.1-ALv2
/** Inspect generator HTML with parse5. Source links resolve against canonical files;
 * generated navigation resolves against the configured site base. No remote crawler. */
import { execFileSync } from 'node:child_process'
import { existsSync, lstatSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { parse, serialize, type DefaultTreeAdapterTypes as Tree } from 'parse5'
import { files } from './artifact.ts'

export { files } from './artifact.ts'

import { address, base, routePath } from './settings.ts'
import { root } from './sources.ts'

export type Page = { file: string; id: string; digest: string }
export function elements(node: Tree.Node): Tree.Element[] {
  const result: Tree.Element[] = []
  if ('tagName' in node) result.push(node)
  if ('childNodes' in node) for (const child of node.childNodes) result.push(...elements(child))
  return result
}
export const attr = (node: Tree.Element, name: string) => node.attrs.find((item) => item.name === name)?.value
export function setAttr(node: Tree.Element, name: string, value: string) {
  const item = node.attrs.find((item) => item.name === name)
  if (item) item.value = value
  else node.attrs.push({ name, value })
}

function repositoryTarget(file: string) {
  let at = root
  for (const part of file.split('/')) {
    if (!part || part === '.' || part === '..') throw new Error(`Invalid source link: ${file}`)
    at = join(at, part)
    if (!existsSync(at) || lstatSync(at).isSymbolicLink())
      throw new Error(`Missing or linked source: ${file}`)
  }
  return at
}
const external = (value: string) => /^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(value)
export function rewriteSourceLinks(directory: string, pages: Page[]) {
  const routes = new Map(pages.map((page) => [page.file, routePath(page.id)]))
  const sha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  for (const page of pages) {
    const html = join(directory, page.id === 'index' ? '' : page.id.replace(/\/index$/, ''), 'index.html')
    if (!existsSync(html)) throw new Error(`Page missing from built site: ${page.file}`)
    const document = parse(readFileSync(html, 'utf8'))
    const content = elements(document).find((node) =>
      attr(node, 'class')?.split(/\s+/).includes('sl-markdown-content'),
    )
    if (!content) throw new Error(`No rendered Markdown in ${page.file}`)
    for (const node of elements(content)) {
      const href = attr(node, 'href')
      if (node.tagName !== 'a' || !href || external(href) || href.startsWith('#')) continue
      const target = new URL(href, `https://source.invalid/${page.file}`)
      const file = decodeURIComponent(target.pathname.slice(1)).replace(/\/$/, '')
      const full = repositoryTarget(file)
      const route = routes.get(file) ?? routes.get(`${file}/README.md`) ?? routes.get(`${file}/index.md`)
      setAttr(
        node,
        'href',
        route
          ? `${route}${target.search}${target.hash}`
          : `https://github.com/cubepals/pryx/${lstatSync(full).isDirectory() ? 'tree' : 'blob'}/${sha}/${file}${target.hash}`,
      )
    }
    writeFileSync(html, serialize(document))
  }
}
export function localTarget(value: string, from: string) {
  if (value.startsWith('data:')) return undefined
  const origin = address.origin ?? 'https://preview.invalid'
  const url = new URL(value, `${origin}${base}${from}`)
  if (url.origin !== origin) return undefined
  if (!url.pathname.startsWith(base)) throw new Error(`Link escapes site base: ${value} in ${from}`)
  let path = decodeURIComponent(url.pathname.slice(base.length))
  if (path.endsWith('/') || !path) path += 'index.html'
  return { path, hash: decodeURIComponent(url.hash.slice(1)), search: url.search }
}
export function checkLinks(directory: string) {
  const errors: string[] = []
  const all = files(directory)
  const available = new Set(all)
  const documents = new Map(
    all
      .filter((file) => file.endsWith('.html'))
      .map((file) => [file, parse(readFileSync(join(directory, file), 'utf8'))]),
  )
  const ids = new Map(
    [...documents].map(([file, doc]) => [
      file,
      new Set(
        elements(doc)
          .map((node) => attr(node, 'id'))
          .filter(Boolean),
      ),
    ]),
  )
  for (const [file, document] of documents)
    for (const node of elements(document)) {
      for (const name of ['href', 'src']) {
        const value = attr(node, name)
        if (!value || value.startsWith('mailto:') || value.startsWith('tel:')) continue
        const target = localTarget(value, file)
        if (!target) {
          if (name === 'src' && ['script', 'iframe'].includes(node.tagName))
            throw new Error(`External executable resource: ${value}`)
          if (node.tagName === 'link' && ['stylesheet', 'preload'].includes(attr(node, 'rel') ?? ''))
            throw new Error(`External style/font resource: ${value}`)
          continue
        }
        if (!available.has(target.path)) errors.push(`Broken link: ${file} -> ${value}`)
        if (target.hash && ids.has(target.path) && !ids.get(target.path)?.has(target.hash))
          errors.push(`Broken anchor: ${file} -> ${value}`)
      }
    }
  if (errors.length) throw new Error(errors.join('\n'))
  return documents.size
}
export function offlineHtml(input: string, file: string) {
  const document = parse(input)
  for (const node of elements(document)) {
    if (node.tagName === 'script' && (attr(node, 'type') === 'module' || attr(node, 'src'))) {
      const parent = node.parentNode
      if (parent) parent.childNodes = parent.childNodes.filter((child) => child !== node)
      continue
    }
    if (node.tagName === 'site-search') {
      for (const button of elements(node).filter((item) => item.tagName === 'button')) {
        setAttr(button, 'disabled', '')
        setAttr(button, 'title', 'Search works when the full static site is served over HTTP.')
      }
    }
    for (const name of ['href', 'src']) {
      const value = attr(node, name)
      if (!value || value.startsWith('mailto:') || value.startsWith('tel:')) continue
      const target = localTarget(value, file)
      if (target)
        setAttr(
          node,
          name,
          `${relative(dirname(file), target.path) || 'index.html'}${target.search}${target.hash ? `#${target.hash}` : ''}`,
        )
    }
  }
  const body = elements(document).find((node) => node.tagName === 'body')
  if (body) {
    const banner = parse(
      '<p style="padding:1rem;background:Canvas;color:CanvasText">Offline review: links and page text work from this file. Serve the full site over HTTP for search and interactive navigation.</p>',
    )
    const paragraph = elements(banner).find((node) => node.tagName === 'p')
    if (paragraph) {
      const main = elements(document).find((node) => node.tagName === 'main') ?? body
      paragraph.parentNode = main
      main.childNodes.unshift(paragraph)
    }
  }
  return serialize(document)
}
