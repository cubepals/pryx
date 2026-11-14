// SPDX-License-Identifier: FSL-1.1-ALv2
/** Site acceptance regressions: a missing or changed canonical page and broken rendered links
 * must fail. Private fixtures exercise symlink refusal and address validation without publishing. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'
import { checkLinks } from '../../site/tools/html.ts'
import { root, site, source, sourceFiles, verifySources } from '../../site/tools/sources.ts'
import { PRYX_VERSION } from '../src/generated/release.ts'
import { siteAddress, welcomeConnectGuides } from '../src/site-address.ts'

test('canonical page coverage and body drift fail the site check', () => {
  const entries = sourceFiles().map((file) => ({
    id: source(file).id,
    body: source(file).body,
    filePath: relative(site, join(root, file)),
  }))
  expect(verifySources(entries)).toHaveLength(entries.length)
  expect(() => verifySources(entries.slice(1))).toThrow('Page missing from site')
  const altered = entries.map((entry, index) =>
    index ? entry : { ...entry, body: `${entry.body}Changed text` },
  )
  expect(() => verifySources(altered)).toThrow('differs from canonical source')
  const first = entries[0]
  if (!first) throw new Error('Expected canonical documentation')
  expect(() => verifySources([...entries, first])).toThrow('repeated page')
})

test('rendered local files and anchors are required', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-site-links-'))
  try {
    writeFileSync(join(directory, 'index.html'), '<a href="missing/">Missing page</a>')
    expect(() => checkLinks(directory)).toThrow('Broken link')
    writeFileSync(join(directory, 'index.html'), '<a href="#missing">Missing anchor</a>')
    expect(() => checkLinks(directory)).toThrow('Broken anchor')
    writeFileSync(join(directory, 'index.html'), '<a href="#present">Here</a><h1 id="present">Present</h1>')
    expect(checkLinks(directory)).toBe(1)
    symlinkSync(join(directory, 'index.html'), join(directory, 'linked.html'))
    expect(() => checkLinks(directory)).toThrow('symlink')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('canonical source walks refuse links before reading them', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-site-source-'))
  try {
    mkdirSync(join(directory, 'docs'))
    writeFileSync(join(directory, 'outside.md'), '# Outside')
    symlinkSync(join(directory, 'outside.md'), join(directory, 'docs/linked.md'))
    expect(() => sourceFiles(directory)).toThrow('source is a symlink')
    rmSync(join(directory, 'docs/linked.md'))
    mkdirSync(join(directory, 'sdk'))
    symlinkSync(join(directory, 'docs'), join(directory, 'sdk/typescript'))
    expect(() => sourceFiles(directory)).toThrow('source is a symlink')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('site addresses preserve base paths and refuse credential-bearing URLs', () => {
  expect(siteAddress()).toEqual({ origin: undefined, prefix: '/', configured: undefined })
  expect(siteAddress('https://docs.example.test/guide').configured).toBe('https://docs.example.test/guide/')
  for (const url of [
    'https://key@docs.example.test/',
    'javascript:alert(1)',
    'https://docs.example.test/?key=secret',
    'https://docs.example.test/#key',
  ])
    expect(() => siteAddress(url)).toThrow()
})

test('welcome guide URLs name existing canonical pages of the running version', () => {
  const ids = new Set(sourceFiles().map((file) => source(file).id))
  expect(welcomeConnectGuides()).toEqual([])
  for (const guide of welcomeConnectGuides('https://docs.example.test/manual/')) {
    expect(ids.has(`connect/${guide.id}`)).toBe(true)
    expect(guide.url).toBe(`https://docs.example.test/manual/v${PRYX_VERSION}/connect/${guide.id}/`)
  }
  try {
    siteAddress('private-not-a-url')
  } catch (error) {
    expect(String(error)).not.toContain('private-not-a-url')
    expect(Object.hasOwn(error as object, 'input')).toBe(false)
  }
})
