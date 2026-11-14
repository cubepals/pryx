// SPDX-License-Identifier: FSL-1.1-ALv2
/** Build with the maintained generator, check all pages/links and export a file-readable review.
 * Parts (tools/): sources.ts owns canonical parity; settings.ts owns version/address;
 * html.ts owns parsed links and export. This command never publishes. */
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { checkLinks, files, offlineHtml, type Page, rewriteSourceLinks } from './html.ts'
import { base, version } from './settings.ts'
import { site } from './sources.ts'

rmSync(join(site, '.astro/collections'), { recursive: true, force: true })
rmSync(join(site, 'dist'), { recursive: true, force: true })
rmSync(join(site, 'review'), { recursive: true, force: true })
const child = Bun.spawn(['bun', 'run', 'build'], {
  cwd: site,
  env: { ...process.env, ASTRO_TELEMETRY_DISABLED: '1' },
  stdout: 'inherit',
  stderr: 'inherit',
})
if (await child.exited) throw new Error('Static site build failed')
const directory = join(site, 'dist', `v${version}`)
const pages: Page[] = JSON.parse(readFileSync(join(site, '.build/sources.json'), 'utf8'))
rewriteSourceLinks(directory, pages)
const count = checkLinks(directory)
const review = join(site, 'review')
cpSync(directory, review, { recursive: true, dereference: false })
for (const file of files(review).filter((file) => file.endsWith('.html'))) {
  const path = join(review, file)
  writeFileSync(path, offlineHtml(readFileSync(path, 'utf8'), file))
}
checkLinks(review)
mkdirSync(join(site, '.build'), { recursive: true })
writeFileSync(join(site, '.build/build.json'), JSON.stringify({ version, base, pages: count }))
console.log(
  `Site: ${pages.length} canonical pages; ${count} HTML documents; local links and anchors pass; file review exported`,
)
