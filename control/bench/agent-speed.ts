// SPDX-License-Identifier: FSL-1.1-ALv2
/** Current agent measurements retain all thirty validated samples. Historical collectors run
 * verbatim from their pinned checkout, outside the current SDK and production source. */

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import type { AddressInfo } from 'node:net'
import { cpus, release, totalmem } from 'node:os'
import { StatusCodes } from 'http-status-codes'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { AppRequests } from '../e2e/app-requests.ts'
import { bin, deployment, ready } from '../e2e/deployment.ts'
import { SECOND_MS } from '../src/durations.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { decodeHandle } from '../src/handle.ts'

const phase = process.argv[2]
if (phase !== 'after') throw new Error('agent-speed.ts after output.json')
const sourceRoot = new URL('../../', import.meta.url).pathname
if (!ready) throw new Error('Agent measurements require real Docker, Postgres, S3 and daemon')
const d = await deployment()
const edge = d.edge
const port = (edge.listeners.get('http:80')?.address() as AddressInfo).port
const origin = `http://127.0.0.1:${port}`
const apps = new AppRequests(origin)
const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
const box = await sdk.sandbox({
  image: 'alpine:pryx-tls',
  resources: { memoryMb: 512 },
  sparePorts: 1,
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
      const response = await apps.get(`http://${host}/`)
      if (response.status === StatusCodes.OK && (await response.text()) === 'ready') return
    } catch {
      /* A read can wait for the already-started listener; a start is never retried. */
    }
    if (performance.now() > end) throw new Error('Exposed server did not return its asserted body')
    await Bun.sleep(SECOND_MS / 1000)
  }
}
try {
  const routeId = `${box.id}:${AGENT_ROUTE_PORT}`
  while (!edge.table.held.has(routeId)) await Bun.sleep(SECOND_MS / 20)
  await box.files.list('.')
  await box.run('true')
  await measure('run_edge', async () => {
    if ((await box.run('true')).code !== 0) throw new Error('Run failed')
  })
  const first: number[] = []
  for (let i = 0; i < N; i++) {
    const start = performance.now(),
      run = box.stream('echo hi'),
      stream = run[Symbol.asyncIterator]()
    const chunk = await stream.next()
    if (chunk.value?.data !== 'hi\n') throw new Error('First byte did not contain the expected output')
    first.push(performance.now() - start)
    while (!(await stream.next()).done) {}
    if ((await run.result()).code !== 0) throw new Error('Output command failed')
  }
  samples.first_byte = first
  await box.write('index.html', 'ready')
  const exposes: number[] = []
  for (let i = 0; i < N; i++) {
    const start = performance.now()
    const process = await box.spawn(['httpd', '-f', '-p', '3000', '-h', '/data'])
    const url = await box.expose(3000)
    await available(new URL(url).hostname)
    exposes.push(performance.now() - start)
    await box.unexpose(3000)
    await process.kill()
    await process.wait()
  }
  samples.start_expose_200 = exposes
  {
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
    await direct.files.list('.')
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
  }
  const result = {
    phase,
    source: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: sourceRoot, encoding: 'utf8' }).stdout.trim(),
    collector: spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).stdout.trim(),
    agentBinarySha256: createHash('sha256')
      .update(readFileSync(nodeSchema['x-pryx-agent'].defaultBinary))
      .digest('hex'),
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
  apps.close()
  await box.delete()
  await d.close()
}
