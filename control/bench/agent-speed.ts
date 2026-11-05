// SPDX-License-Identifier: FSL-1.1-ALv2
/** Paired agent measurements use the selected source checkout and its real daemon. Unsupported
 * baseline agent surfaces have n=0; every available path retains all 30 samples and its cohort. */

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import type { AddressInfo } from 'node:net'
import { cpus, release, totalmem } from 'node:os'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { StatusCodes } from 'http-status-codes'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { SECOND_MS } from '../src/durations.ts'

const phase = process.argv[2]
if (phase !== 'before' && phase !== 'after') throw new Error('agent-speed.ts before|after output.json')
const sourceRoot = resolve(process.env.PRYX_SPEED_SOURCE ?? new URL('../../', import.meta.url).pathname)
const source = pathToFileURL(`${sourceRoot}/`)
const { deployment, ready, bin } = (await import(
  new URL('control/e2e/deployment.ts', source).href
)) as typeof import('../e2e/deployment.ts')
const { startEdge } = (await import(
  new URL('edge/src/agent.ts', source).href
)) as typeof import('../../edge/src/agent.ts')
const { edgeConfig } = (await import(
  new URL('edge/src/config.ts', source).href
)) as typeof import('../../edge/src/config.ts')
const { Pryx } = (await import(
  new URL('sdk/typescript/src/index.ts', source).href
)) as typeof import('../../sdk/typescript/src/index.ts')
const { decodeHandle } = (await import(
  new URL('control/src/handle.ts', source).href
)) as typeof import('../src/handle.ts')
if (!ready) throw new Error('Agent measurements require real Docker, Postgres, S3 and daemon')
const d = await deployment({ agent: phase === 'after' })
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
const port = (edge.listeners.get('http:80')?.address() as AddressInfo).port
const origin = `http://127.0.0.1:${port}`
if (phase === 'after' && d.control.edge.agents) d.control.edge.agents.localOrigin = origin
const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
const box = await sdk.sandbox({
  image: 'alpine:pryx-tls',
  resources: { memoryMb: 512 },
  ...(phase === 'before'
    ? { ports: [{ name: 'web', port: 3000, audience: 'public' as const }] }
    : { sparePorts: 1 }),
})
const samples: Record<string, number[]> = {}
const absent: Record<string, string> = {}
const N = 30,
  DATA_BYTES = 100_000_000
async function measure(name: string, operation: (index: number) => Promise<void>) {
  const values = samples[name] ?? []
  samples[name] = values
  for (let i = 0; i < N; i++) {
    const start = performance.now()
    await operation(i)
    values.push(performance.now() - start)
  }
}
async function available(host: string) {
  const end = performance.now() + SECOND_MS
  while (true) {
    try {
      const response = await fetch(origin, { headers: { host } })
      if (response.status === StatusCodes.OK && (await response.text()) === 'ready') return
    } catch {
      /* A read can wait for the already-started listener; a start is never retried. */
    }
    if (performance.now() > end) throw new Error('Exposed server did not return its asserted body')
    await Bun.sleep(SECOND_MS / 1000)
  }
}
try {
  const routeId = phase === 'after' ? `${box.id}:_pryx` : `${box.id}:web`
  while (!edge.table.held.has(routeId)) await Bun.sleep(SECOND_MS / 20)
  if (phase === 'after') await box.agent.token()
  await box.run('true')
  await measure('run_edge', async () => {
    if ((await box.run('true')).code !== 0) throw new Error('Run failed')
  })
  const first: number[] = []
  for (let i = 0; i < N; i++) {
    const start = performance.now(),
      run = box.run('echo hi'),
      stream = run.stream()
    const chunk = await stream.next()
    if (chunk.value?.data !== 'hi\n') throw new Error('First byte did not contain the expected output')
    first.push(performance.now() - start)
    while (!(await stream.next()).done) {}
    if ((await run).code !== 0) throw new Error('Output command failed')
  }
  samples.first_byte = first
  await box.write('index.html', 'ready')
  const exposes: number[] = []
  for (let i = 0; i < N; i++) {
    const start = performance.now()
    if (phase === 'after') {
      const process = await box.start(['httpd', '-f', '-p', '3000', '-h', '/data'])
      const url = await box.expose(3000)
      await available(new URL(url).hostname)
      exposes.push(performance.now() - start)
      await box.unexpose(3000)
      await process.kill()
      await process.wait()
    } else {
      const started = await box.run('httpd -f -p 3000 -h /data </dev/null >/dev/null 2>&1 & echo $!')
      if (started.code !== 0) throw new Error('Baseline server start failed')
      await available(`${box.id}.pryx.test`)
      exposes.push(performance.now() - start)
      if ((await box.run(['kill', started.stdout.trim()])).code !== 0)
        throw new Error('Baseline server cleanup failed')
    }
  }
  samples.start_expose_200 = exposes
  if (phase === 'after') {
    const row = await d.control.sandboxes.record(box.id)
    if (!row.handle) throw new Error('Missing agent home')
    const endpoint = decodeHandle(row.handle).agent?.port.endpoints[0]
    if (!endpoint) throw new Error('Missing loopback agent endpoint')
    const directSdk = new Pryx({
      url: d.control.apiUrl,
      key: d.credential.key,
      fetch: (async (input, init) => {
        const response = await fetch(input, init)
        return String(input).endsWith('/token') && response.ok
          ? Response.json({ ...(await response.json()), url: `http://${endpoint}` })
          : response
      }) as typeof fetch,
    })
    const direct = await directSdk.sandbox(box.id)
    await direct.agent.token()
    await measure('run_node_loopback', async () => {
      if ((await direct.run('true')).code !== 0) throw new Error('Loopback run failed')
    })
    const terminal = await box.terminal(),
      stream = terminal.stream()
    const roundtrips: number[] = []
    try {
      for (let i = 0; i < N; i++) {
        const start = performance.now()
        terminal.stdin.write(`printf 'round:%s\\n' ${i}\n`)
        let output = ''
        while (!output.includes(`round:${i}\r\n`)) {
          const part = await stream.next()
          if (part.done) throw new Error('Terminal ended')
          output += part.value.data
        }
        roundtrips.push(performance.now() - start)
      }
    } finally {
      terminal.close()
      await stream.return(undefined)
    }
    samples.terminal_roundtrip = roundtrips
    const chunk = new Uint8Array(1024 * 1024).fill(42)
    await measure('write_100mb', async () => {
      await box.write(
        '100mb',
        (async function* () {
          let left = DATA_BYTES
          while (left) {
            const part = chunk.subarray(0, Math.min(left, chunk.length))
            yield part
            left -= part.length
          }
        })(),
      )
    })
    await measure('read_100mb', async () => {
      let count = 0
      for await (const bytes of await box.readStream('100mb')) {
        if (bytes[0] !== 42 || bytes[bytes.length - 1] !== 42) throw new Error('File content changed')
        count += bytes.length
      }
      if (count !== DATA_BYTES) throw new Error('File was truncated')
    })
  } else
    for (const name of ['run_node_loopback', 'terminal_roundtrip', 'write_100mb', 'read_100mb'])
      absent[name] = 'The baseline has no agent, PTY or streamed file transport; n=0'
  const result = {
    phase,
    source: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: sourceRoot, encoding: 'utf8' }).stdout.trim(),
    collector: spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).stdout.trim(),
    agentBinarySha256:
      phase === 'after'
        ? createHash('sha256').update(readFileSync(nodeSchema['x-pryx-agent'].defaultBinary)).digest('hex')
        : null,
    binarySha256: createHash('sha256').update(readFileSync(bin)).digest('hex'),
    host: {
      cpu: cpus()[0]?.model,
      cores: cpus().length,
      kernel: release(),
      memoryBytes: totalmem(),
      bun: Bun.version,
    },
    dataset: { bytes: DATA_BYTES, byte: 42 },
    absent,
    measurements: Object.fromEntries(
      Object.entries(samples).map(([name, raw]) => {
        const sorted = [...raw].sort((a, b) => a - b)
        return [
          name,
          {
            n: raw.length,
            p50Ms: sorted[Math.floor(sorted.length / 2)],
            p90Ms: sorted[Math.ceil(sorted.length * 0.9) - 1],
            samplesMs: raw,
          },
        ]
      }),
    ),
  }
  writeFileSync(process.argv[3] ?? `/tmp/agent-speed-${phase}.json`, JSON.stringify(result, null, 2))
  console.log(JSON.stringify(result))
} finally {
  await box.delete()
  await edge.stop()
  await d.close()
}
