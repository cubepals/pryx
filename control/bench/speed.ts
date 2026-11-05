// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Real one-box latency samples for K9. This collector requires Docker/Postgres/S3; it never substitutes a fake runtime or invents a sample. */

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { cpus, release } from 'node:os'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import type { Pryx } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import type { NodeClient } from '../src/node-client.ts'
import type { PublicModel } from '../src/public-model.ts'

const N = 30
const DATA_BYTES = 100_000_000
const IMAGE = 'alpine:3.22'
const COLD_IMAGE = 'mirror.gcr.io/library/alpine:3.22'
const phase = process.argv[2]
if (phase !== 'before' && phase !== 'after') throw new Error('speed.ts before|after [output.json]')
const collectorRoot = new URL('../../', import.meta.url).pathname
const sourceRoot = resolve(process.env.PRYX_SPEED_SOURCE ?? collectorRoot)
const source = pathToFileURL(`${sourceRoot}/`)
const { deployment, ready, bin } = (await import(
  new URL('control/e2e/deployment.ts', source).href
)) as typeof import('../e2e/deployment.ts')
const { nodeSummaries } = (await import(
  new URL('control/src/registry.ts', source).href
)) as typeof import('../src/registry.ts')
const { startEdge } = (await import(
  new URL('edge/src/agent.ts', source).href
)) as typeof import('../../edge/src/agent.ts')
const { edgeConfig } = (await import(
  new URL('edge/src/config.ts', source).href
)) as typeof import('../../edge/src/config.ts')
const { Pryx: MeasuredPryx } = (await import(new URL('sdk/typescript/src/index.ts', source).href)) as {
  Pryx: typeof Pryx
}
if (!ready) throw new Error('Speed measurement requires the real daemon, Docker, Postgres and S3')
const d = await deployment({ allowedImages: ['alpine:', 'mirror.gcr.io/library/alpine:'] })
const client = new MeasuredPryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key })
const edge = await startEdge({
  ...edgeConfig({
    PRYX_CONTROL_URL: d.control.apiUrl,
    PRYX_EDGE_TOKEN: d.control.config.edgeToken,
    PRYX_EDGE_HOST: '127.0.0.1',
  }),
  httpPort: 0,
  tlsPort: 0,
  metricsPort: 0,
})
const spec = {
  image: IMAGE,
  entrypoint: [
    'sh',
    '-c',
    'trap \'kill "$child"; exit 0\' TERM INT; tail -f /dev/null & child=$!; wait "$child"',
  ],
  resources: { memoryMb: 128 },
}
const profiles: Record<string, number[]> = {}
const original = d.control.nodes.call.bind(d.control.nodes)
d.control.nodes.call = (async (...args: Parameters<NodeClient['call']>) => {
  const started = performance.now()
  try {
    return await original(...args)
  } finally {
    const key = `${args[1]} ${args[2].split('?')[0]?.split('/')[4] ?? 'workload'}`
    profiles[key] ??= []
    profiles[key].push(performance.now() - started)
  }
}) as NodeClient['call']
type Measurement = { n: number; p50Ms: number; p90Ms: number; samplesMs: number[] }
const results: Record<string, Measurement> = {}
function summarize(samplesMs: number[]): Measurement {
  const sorted = [...samplesMs].sort((a, b) => a - b)
  if (!sorted.length) throw new Error('No samples')
  return {
    n: samplesMs.length,
    p50Ms: sorted[Math.ceil(sorted.length * 0.5) - 1] as number,
    p90Ms: sorted[Math.ceil(sorted.length * 0.9) - 1] as number,
    samplesMs,
  }
}
async function sample<T>(
  name: string,
  action: () => Promise<T>,
  cleanup?: (value: T) => Promise<unknown>,
  setup?: () => Promise<void>,
) {
  const samples: number[] = []
  for (let index = 0; index < N; index++) {
    if (setup) await setup()
    const started = performance.now()
    const value = await action()
    samples.push(performance.now() - started)
    if (cleanup) await cleanup(value)
  }
  results[name] = summarize(samples)
  console.log(`SPEED_CASE ${phase} ${name} ${JSON.stringify(results[name])}`)
}
async function exec(id: string, command: string[], timeout = 30) {
  let code: number | null = null
  for await (const event of client.exec(id, { command, timeout }, crypto.randomUUID()))
    if (event.type === 'exit') code = event.code
  if (code !== 0) throw new Error(`Benchmark command failed with ${code}`)
}
async function running<T extends { id: string; state: string }>(box: T) {
  if (box.state !== 'running') throw new Error(`Create returned ${box.state}, expected running`)
  return box
}
try {
  const nodes = await nodeSummaries(d.control.db, {
    suspectSeconds: d.control.config.suspectSeconds,
    unavailableSeconds: d.control.config.unavailableSeconds,
  })
  const daemonFeatures = nodes.map((node) => ({ id: node.id, features: node.features }))
  if (nodes.length !== 1 || nodes[0]?.features.includes('exec-stream') !== (phase === 'after'))
    throw new Error(`The ${phase} daemon advertises the wrong features: ${JSON.stringify(daemonFeatures)}`)
  const daemonSha256 = createHash('sha256').update(readFileSync(bin)).digest('hex')
  console.log(`SPEED_DAEMON ${phase} ${JSON.stringify({ daemonSha256, daemonFeatures })}`)
  await Promise.race([
    edge.poll.firstHold,
    Bun.sleep(MINUTE_MS).then(() => {
      throw new Error('Edge did not initialize')
    }),
  ])
  const box = await running(await client.make(spec, crypto.randomUUID()))
  try {
    await exec(box.id, ['sh', '-c', 'true'])
    const handle = phase === 'after' ? await client.sandbox(box.id) : null
    await sample('api_exec', async () => {
      if (!handle) return exec(box.id, ['sh', '-c', 'true'])
      const result = await handle.run('true')
      if (result.code !== 0 || result.timedOut) throw new Error('Handle command failed')
    })
    await sample('control_exec', async () => {
      const result = await d.control.sandboxes.exec(box.id, ['sh', '-c', 'true'], 30, 'default')
      if (result.exitCode !== 0) throw new Error('Control exec failed')
    })
    const wakeSamples: number[] = []
    for (let index = 0; index < N; index++) {
      const slept = await client.action(box.id, 'sleep')
      if (slept.tier !== 'suspended') throw new Error('Suspension unavailable')
      const started = performance.now()
      const woke = await client.action(box.id, 'wake')
      if (woke.tier !== 'resumed') throw new Error('Wake did not resume')
      wakeSamples.push(performance.now() - started)
    }
    results.wake_suspended = summarize(wakeSamples)
    const startSamples: number[] = []
    for (let index = 0; index < N; index++) {
      await client.action(box.id, 'stop')
      const started = performance.now()
      const startedBox = await client.action(box.id, 'start')
      if (startedBox.sandbox.state !== 'running') throw new Error('Start did not reach running')
      startSamples.push(performance.now() - started)
    }
    results.start_stopped = summarize(startSamples)
    await exec(box.id, ['sh', '-c', `head -c ${DATA_BYTES} /dev/zero > /data/speed.bin`], 600)
    await sample(
      'snapshot_100mb',
      () => client.snapshot(box.id),
      async (snapshot) => {
        await d.control.runtime.upload(snapshot.id, true)
        return client.deleteSnapshot(box.id, snapshot.id)
      },
    )
    const snapshot = await client.snapshot(box.id)
    try {
      await sample(
        'fork_100mb',
        async () => running(await client.fork(snapshot.id, {}, crypto.randomUUID())),
        (fork) => client.delete(fork.id),
      )
    } finally {
      await client.deleteSnapshot(box.id, snapshot.id)
    }
  } finally {
    await client.delete(box.id)
  }
  await sample(
    'create_image_cached',
    async () => running(await client.make(spec, crypto.randomUUID())),
    (box) => client.delete(box.id),
  )
  await sample(
    'create_image_cold',
    async () => running(await client.make({ ...spec, image: COLD_IMAGE }, crypto.randomUUID())),
    (box) => client.delete(box.id),
    async () => {
      const references = [IMAGE, COLD_IMAGE].filter(
        (image) => spawnSync('docker', ['image', 'inspect', image], { stdio: 'ignore' }).status === 0,
      )
      if (!references.length) throw new Error('The preceding sample did not leave its image reference')
      spawnSync('docker', ['image', 'rm', '-f', ...references], { stdio: 'ignore' })
      if (
        references.some(
          (image) => spawnSync('docker', ['image', 'inspect', image], { stdio: 'ignore' }).status === 0,
        )
      )
        throw new Error('Could not clear the measured image references')
    },
  )
  const tagged = spawnSync('docker', ['tag', COLD_IMAGE, IMAGE])
  if (tagged.status !== 0) throw new Error('Could not restore the test image tag')
  const template = (await client.template({ name: `speed-${phase}`, spec })) as Awaited<
    ReturnType<PublicModel['template']>
  >
  if (
    !template ||
    !template.copies.length ||
    !template.copies.every((copy) => (copy as { ready: boolean }).ready)
  )
    throw new Error('Template did not warm')
  await sample(
    'create_template',
    async () => running(await client.make({ template: template.id }, crypto.randomUUID())),
    (box) => client.delete(box.id),
  )
  const report = {
    phase,
    n: N,
    at: new Date().toISOString(),
    commit: spawnSync('git', ['-C', sourceRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).stdout.trim(),
    collectorCommit: spawnSync('git', ['-C', collectorRoot, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
    }).stdout.trim(),
    machine: {
      platform: process.platform,
      arch: process.arch,
      kernel: release(),
      cpu: cpus()[0]?.model,
      cores: cpus().length,
      bun: Bun.version,
    },
    dataBytes: DATA_BYTES,
    daemonSha256,
    daemonFeatures,
    image: IMAGE,
    coldImage: COLD_IMAGE,
    coldDefinition:
      'Image references removed before each sample; shared layer and OS caches may remain. Cleanup is excluded.',
    results,
    nodeCalls: Object.fromEntries(
      Object.entries(profiles).map(([name, samples]) => [name, summarize(samples)]),
    ),
  }
  const output = `${JSON.stringify(report, null, 2)}\n`
  writeFileSync(process.argv[3] ?? `speed-${phase}.json`, output)
  console.log(`PRYX_SPEED ${JSON.stringify(report)}`)
} finally {
  await edge.stop()
  await d.close()
}
