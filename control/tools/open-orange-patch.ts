// SPDX-License-Identifier: FSL-1.1-ALv2
/** Check the prepared OpenOrange registration against exact upstream files; never publish it.
 * Originals live only in ignored output, checked by digest before the patch is applied and bundled.
 */
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'

const root = resolve(import.meta.dir, '../..')
const output = join(root, 'target/k30-open-orange')
const manifest = JSON.parse(readFileSync(join(root, 'docs/upstream/open-orange/source.json'), 'utf8')) as {
  commit: string
  files: Record<string, string>
}
for (const [path, digest] of Object.entries(manifest.files)) {
  const target = join(output, path)
  mkdirSync(dirname(target), { recursive: true })
  const child = Bun.spawn(
    [
      'curl',
      '--config',
      join(root, '.github/download.curlrc'),
      '-fsSL',
      `https://raw.githubusercontent.com/rivet-dev/open-orange/${manifest.commit}/${path}`,
      '-o',
      target,
    ],
    { stdout: 'inherit', stderr: 'inherit' },
  )
  if (await child.exited) throw new Error(`OpenOrange source fetch failed: ${path}`)
  if (createHash('sha256').update(readFileSync(target)).digest('hex') !== digest)
    throw new Error(`OpenOrange source digest changed: ${path}`)
}
const applied = Bun.spawn(
  ['git', 'apply', '--directory=target/k30-open-orange', 'docs/upstream/open-orange/change.patch'],
  {
    cwd: root,
    stdout: 'inherit',
    stderr: 'inherit',
  },
)
if (await applied.exited) throw new Error('Prepared OpenOrange patch no longer applies')
const provider = join(output, 'packages/runtime-node/src/modules/cloudflare-sandbox/providers.ts')
const built = await Bun.build({
  entrypoints: [provider],
  packages: 'external',
  target: 'bun',
  outdir: join(output, 'built'),
})
if (!built.success) throw new AggregateError(built.logs, 'OpenOrange provider bridge did not build')
writeFileSync(
  join(output, 'result.json'),
  JSON.stringify({
    commit: manifest.commit,
    patchApplied: true,
    provider: join(output, 'built/providers.js'),
  }),
)
console.log(`OpenOrange ${manifest.commit}: patch applies and provider bridge builds`)
