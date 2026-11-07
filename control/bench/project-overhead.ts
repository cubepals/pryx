// SPDX-License-Identifier: FSL-1.1-ALv2
/** Measure project admission overhead with real Postgres and the established in-memory node. This is not container latency. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { StatusCodes } from 'http-status-codes'

const phase = process.argv[2]
if (phase !== 'before' && phase !== 'after') throw new Error('project-overhead.ts before|after output.json')
const sourceRoot = resolve(process.env.PRYX_PROJECT_SOURCE ?? new URL('../../', import.meta.url).pathname)
const { publicContext } = (await import(
  new URL('control/test/public-context.ts', pathToFileURL(`${sourceRoot}/`)).href
)) as typeof import('../test/public-context.ts')
const c = await publicContext(),
  samplesMs: number[] = []
try {
  for (let index = 0; index <= 30; index++) {
    const started = performance.now()
    const response = await c.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      resources: { memoryMb: 128 },
    })
    if (response.status !== StatusCodes.CREATED) throw new Error(await response.text())
    const box = (await response.json()) as { id: string }
    if (index) samplesMs.push(performance.now() - started)
    const deleted = await c.request(`/sandboxes/${box.id}`, 'DELETE')
    if (!deleted.ok) throw new Error(await deleted.text())
  }
  const sorted = [...samplesMs].sort((a, b) => a - b)
  const hash = createHash('sha256')
  for (const path of [
    ...new Bun.Glob('control/{src,test}/**/*.{ts,sql,json}').scanSync({ cwd: sourceRoot, onlyFiles: true }),
  ].sort())
    hash.update(path).update(readFileSync(resolve(sourceRoot, path)))
  const report = {
    phase,
    commit: spawnSync('git', ['-C', sourceRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).stdout.trim(),
    uncommittedChanges:
      spawnSync('git', ['-C', sourceRoot, 'status', '--porcelain'], { encoding: 'utf8' }).stdout.trim()
        .length > 0,
    sourceSha256: hash.digest('hex'),
    scope: 'real Postgres public API with existing in-memory node double; not Docker latency',
    n: samplesMs.length,
    p50Ms: sorted[14],
    p90Ms: sorted[26],
    samplesMs,
  }
  writeFileSync(process.argv[3] ?? `projects-${phase}.json`, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
} finally {
  await c.close()
}
