// SPDX-License-Identifier: FSL-1.1-ALv2
/** Paired fresh-process and persistent-shell latency on the same wasm host. Each sample checks
 * exact output/status. The baseline has no shell session, which is recorded as unavailable. */
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { cpus, release, totalmem } from 'node:os'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { REPETITIONS } from './tiers/report.ts'

const [phase, outputPath] = process.argv.slice(2)
if (!['before', 'after'].includes(phase ?? '') || !outputPath)
  throw new Error('field-speed.ts before|after output.json')
const root = resolve(process.env.PRYX_FIELD_SOURCE_ROOT ?? new URL('../../', import.meta.url).pathname)
const source = Bun.spawnSync(['git', '-C', root, 'rev-parse', 'HEAD']).stdout.toString().trim()
const dirty = Boolean(Bun.spawnSync(['git', '-C', root, 'status', '--porcelain']).stdout.toString().trim())
const { agentDeployment } = await import(pathToFileURL(resolve(root, 'control/e2e/agent-context.ts')).href)
const c = await agentDeployment('wasm')
const box = await c.sandbox()
function summary(samplesMs: number[]) {
  const sorted = [...samplesMs].sort((a, b) => a - b)
  return {
    n: samplesMs.length,
    p50Ms: sorted[Math.ceil(samplesMs.length * 0.5) - 1],
    p90Ms: sorted[Math.ceil(samplesMs.length * 0.9) - 1],
    samplesMs,
  }
}
function exact(result: { code: number | null; stdout: string; stderr: string; timedOut: boolean }) {
  if (result.code !== 0 || result.stdout !== '' || result.stderr !== '' || result.timedOut)
    throw new Error('Measured command result differs')
}
try {
  exact(await box.run('true'))
  const fresh: number[] = []
  for (let i = 0; i < REPETITIONS; i++) {
    const start = performance.now()
    exact(await box.run('true'))
    fresh.push(performance.now() - start)
  }
  const separate = await box.run('printf out; printf err >&2; exit 7')
  if (separate.code !== 7 || separate.stdout !== 'out' || separate.stderr !== 'err')
    throw new Error('Fresh streams/status differ')
  let persistentShell: unknown = { n: 0, reason: 'The K15 surface has no shell session method' }
  if (phase === 'before') {
    if ('shell' in box) throw new Error('Baseline unexpectedly has shell sessions')
  } else {
    const start = performance.now()
    const shell = await box.shell()
    const openMs = performance.now() - start
    try {
      exact(await shell.run('true'))
      const samples: number[] = []
      for (let i = 0; i < REPETITIONS; i++) {
        const start = performance.now()
        exact(await shell.run('true'))
        samples.push(performance.now() - start)
      }
      await shell.run('mkdir -p shell-state; cd shell-state; export KEPT=yes')
      const state = await shell.run('printf "$KEPT"; pwd; printf err >&2; false')
      if (
        !state.stdout.startsWith('yes') ||
        !state.stdout.endsWith('/shell-state\n') ||
        state.stderr !== 'err' ||
        state.code !== 1
      )
        throw new Error('Shell state, stderr or status differs')
      persistentShell = {
        mechanism: 'pipe-driven read/eval with independent nonce markers on stdout and stderr',
        openMs,
        ...summary(samples),
        semantics: { cwd: true, environment: true, separateStreams: true, exactExitCode: true },
      }
    } finally {
      await shell.close()
    }
  }
  const daemon = process.env.PRYXD_BIN ?? resolve(root, 'target/debug/pryxd')
  const report = {
    phase,
    source,
    dirty,
    tier: 'wasm',
    at: new Date().toISOString(),
    daemonSha256: createHash('sha256').update(readFileSync(daemon)).digest('hex'),
    host: {
      cpu: cpus()[0]?.model,
      cores: cpus().length,
      kernel: release(),
      memoryBytes: totalmem(),
      bun: Bun.version,
    },
    fresh: summary(fresh),
    persistentShell,
  }
  writeFileSync(outputPath, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
} finally {
  await box.delete()
  await c.close()
}
