// SPDX-License-Identifier: FSL-1.1-ALv2
/** Maintainer opt-in command. No destination is contacted or written while the switch is off. */

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { PRYX_VERSION } from '../control/src/generated/release.ts'
import { files, sourceGit } from './tools/artifact.ts'
import { assertRelease, publishSite, type SiteManifest } from './tools/publish.ts'

if (process.env.PRYX_SITE_PUBLISH !== '1') {
  console.log('Site publication is off. A maintainer may enable PRYX_SITE_PUBLISH=1.')
  process.exit(0)
}
const site = dirname(fileURLToPath(import.meta.url))
const root = dirname(site)
const url = process.env.PRYX_SITE_URL
const webroot = process.env.PRYX_SITE_PUBLISH_DIR
if (!url || !webroot)
  throw new Error('Set PRYX_SITE_URL and PRYX_SITE_PUBLISH_DIR for the existing static host')
const commit = sourceGit(root, 'rev-parse', 'HEAD')
assertRelease(root, PRYX_VERSION, commit)
const tracked = new Set(sourceGit(root, 'ls-files', '-z').split('\0'))
if (files(join(site, 'public')).some((file) => !tracked.has(`site/public/${file}`)))
  throw new Error('Publish only tracked public assets')
const build = Bun.spawn([process.execPath, 'site/check.ts'], {
  cwd: root,
  stdout: 'inherit',
  stderr: 'inherit',
})
if (await build.exited) throw new Error('Site checks failed; publication refused')
const { sourceFiles } = await import('./tools/sources.ts')
if (sourceFiles().some((file) => !tracked.has(file)))
  throw new Error('Publish only tracked canonical sources')
const manifest: SiteManifest = JSON.parse(readFileSync(join(site, '.build/build.json'), 'utf8'))
console.log(
  publishSite({ repository: root, artifact: join(site, 'dist', `v${PRYX_VERSION}`), webroot, url, manifest }),
)
