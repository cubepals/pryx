// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Root/cgroupfs CI only: cut just pryxd's store traffic, measure kernel stops, then prove CAS-fenced recovery on a second node. */
import { expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { constants } from 'node:os'
import { Pryx } from '@cubepals/pryx'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { HOUR_MS, MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { one } from '../src/sql.ts'
import { S3LeaseBucket } from '../src/store-lease-bucket.ts'
import { deployment, LEASE_TEST, ready } from './deployment.ts'
import { sleep, until } from './waits.ts'

const enabled = ready && process.env.PRYX_LEASE_E2E === '1'
const root = (...args: string[]) => {
  const result = spawnSync('sudo', ['-n', ...args], { encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`Root test command failed: ${args[0]}`)
  return result.stdout.trim()
}
const storeRule = (port: number) => [
  'OUTPUT',
  '-p',
  'tcp',
  '-d',
  '127.0.0.1',
  '--dport',
  String(port),
  '-m',
  'owner',
  '--uid-owner',
  '0',
  '-j',
  'REJECT',
]

test.skipIf(!enabled)(
  'a store cut stops leased sandboxes and only expired CAS proofs permit another home',
  async () => {
    let requests = 0
    const proxy = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      async fetch(request) {
        const url = new URL(request.url),
          upstream = new URL(process.env.S3_TEST_ENDPOINT ?? 'http://127.0.0.1:9000')
        if (url.pathname.includes('/store-leases/v1/') && !url.pathname.includes('/probe/')) requests++
        upstream.pathname = url.pathname
        upstream.search = url.search
        try {
          return await fetch(upstream, {
            method: request.method,
            headers: request.headers,
            body: ['GET', 'HEAD'].includes(request.method) ? undefined : request.body,
            redirect: 'manual',
          })
        } catch {
          return new Response('Store proxy failed', { status: StatusCodes.BAD_GATEWAY })
        }
      },
    })
    const rule = storeRule(proxy.port ?? 0)
    const d = await deployment({ storeLease: true, nodes: 2, runtimeEndpoint: proxy.url.toString() }),
      sdk = new Pryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key })
    const samples: number[] = []
    const costs: { observedMs: number; requests: number; normalizedPerHour: number }[] = []
    const bucket = new S3LeaseBucket(d.control.config.s3)
    let cut = false
    try {
      for (let index = 0; index < 3; index++) {
        const made = await sdk.make({
          image: 'alpine:3.22',
          resources: { memoryMb: 128 },
          entrypoint: [
            'sh',
            '-c',
            'trap "exit 0" TERM; echo lease-data > /data/message; while :; do sleep 1; done',
          ],
        })
        try {
          const row = await d.control.sandboxes.record(made.id),
            ref = decodeHandle(row.handle as string)
          const snapshot = await sdk.snapshot(made.id)
          await d.control.runtime.upload(snapshot.id)
          const before = requests,
            windowStart = performance.now()
          await sleep(15 * SECOND_MS)
          const observedMs = performance.now() - windowStart,
            count = requests - before
          costs.push({
            observedMs,
            requests: count,
            normalizedPerHour: (count * HOUR_MS) / observedMs,
          })
          const ids = root('docker', 'ps', '-q', '--filter', `label=blocklyd.workload=${made.id}`)
            .split(/\s+/)
            .filter(Boolean)
          if (ids.length !== 1) throw new Error('Expected exactly one running source copy')
          root('iptables', '-I', ...rule)
          cut = true
          const started = performance.now()
          await until(
            'leased cgroup stopped',
            async () => root('docker', 'inspect', '-f', '{{.State.Running}}', ids[0] ?? '') === 'false',
            (LEASE_TEST.seconds + 2) * SECOND_MS,
          )
          const elapsed = performance.now() - started
          samples.push(elapsed)
          console.info(
            JSON.stringify({
              case: 'store-cut',
              sample: index,
              stoppedAfterMs: elapsed,
              termSeconds: LEASE_TEST.seconds,
            }),
          )
          expect(elapsed).toBeLessThan(LEASE_TEST.seconds * SECOND_MS)
          // Control still reads the store as the unprivileged runner, while both root nodes cannot renew.
          await until(
            'published lease actually expires',
            async () => {
              const r = await bucket.read(d.control.storeLease?.key(made.id) ?? '')
              return r.object && r.now >= r.object.expiresAtMs
            },
            (LEASE_TEST.seconds + 2) * SECOND_MS,
          )
          // Keep both nodes cut until the old generation is conditionally fenced and its proof persisted.
          await d.control.storeLease?.tick()
          const proof = await one(
            d.control.db,
            sql`SELECT 1 FROM store_lease_fences WHERE workload=${made.id} AND epoch=${ref.epoch}`,
          )
          expect(proof).not.toBeNull()
          root('iptables', '-D', ...rule)
          cut = false
          await d.control.storeLease?.tick()
          await d.control.sandboxes.wake(made.id)
          const recovered = decodeHandle((await d.control.sandboxes.record(made.id)).handle as string)
          expect(recovered.epoch).toBeGreaterThan(ref.epoch)
          expect(recovered.node).not.toBe(ref.node)
          expect(
            await Array.fromAsync(sdk.exec(made.id, { command: ['cat', '/data/message'] })),
          ).toContainEqual(expect.objectContaining({ type: 'stdout', data: 'lease-data\n' }))
        } finally {
          if (cut) {
            root('iptables', '-D', ...rule)
            cut = false
          }
          await sdk.delete(made.id)
        }
      }
      const sorted = [...samples].sort((a, b) => a - b)
      const report = {
        fault: 'root node store TCP rejection',
        n: samples.length,
        termSeconds: LEASE_TEST.seconds,
        renewSeconds: LEASE_TEST.renewSeconds,
        samplesMs: samples,
        requestCosts: costs,
        p50Ms: sorted[Math.floor(sorted.length / 2)],
        p90Ms: sorted[Math.ceil(sorted.length * 0.9) - 1],
      }
      console.info(JSON.stringify(report))
      writeFileSync(
        process.env.PRYX_LEASE_MEASUREMENTS ?? '/tmp/pryx-store-lease-measurements.json',
        JSON.stringify(report, null, 2),
      )
    } finally {
      if (cut) root('iptables', '-D', ...rule)
      await d.close()
      proxy.stop(true)
    }
  },
  6 * MINUTE_MS,
)

test.skipIf(!enabled)(
  'a stopped daemon cannot stop its independent lease watchdog',
  async () => {
    const d = await deployment({ storeLease: true }),
      sdk = new Pryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key })
    let pid: string | undefined
    try {
      const made = await sdk.make({
        image: 'alpine:3.22',
        resources: { memoryMb: 128 },
        entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
      })
      const ids = root('docker', 'ps', '-q', '--filter', `label=blocklyd.workload=${made.id}`)
        .split(/\s+/)
        .filter(Boolean)
      const path = d.children[0]?.path
      if (!path || ids.length !== 1) throw new Error('Missing daemon/source container')
      const patternQuote = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
      pid = root(
        'pgrep',
        '-f',
        `^${patternQuote(process.env.PRYXD_BIN ?? '')} serve --config ${patternQuote(path)}`,
      )
      root('kill', `-${constants.signals.SIGSTOP}`, pid)
      const started = performance.now()
      await until(
        'independent watchdog stopped sandbox',
        async () => root('docker', 'inspect', '-f', '{{.State.Running}}', ids[0] ?? '') === 'false',
        (LEASE_TEST.seconds + 2) * SECOND_MS,
      )
      const elapsed = performance.now() - started
      expect(elapsed).toBeLessThan(LEASE_TEST.seconds * SECOND_MS)
      console.info(
        JSON.stringify({
          case: 'daemon-SIGSTOP',
          n: 1,
          stoppedAfterMs: elapsed,
          termSeconds: LEASE_TEST.seconds,
        }),
      )
    } finally {
      if (pid) root('kill', `-${constants.signals.SIGCONT}`, pid)
      await d.close()
    }
  },
  2 * MINUTE_MS,
)
