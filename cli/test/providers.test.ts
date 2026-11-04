// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Operator CLI provider commands traverse authenticated HTTP and durable Postgres identities. */
import { describe, expect, test } from 'bun:test'
import { Autoscale } from '../../control/src/autoscale.ts'
import { THRESHOLDS } from '../../control/src/health.ts'
import { createOperatorApi } from '../../control/src/operator-api.ts'
import { withProviders } from '../../control/src/provider-api.ts'
import { ProviderLedger } from '../../control/src/provider-ledger.ts'
import { FakeProvider } from '../../control/src/providers/fake.ts'
import type { Shape } from '../../control/src/providers/provider.ts'
import { heartbeat } from '../../control/src/registry.ts'
import { beat, enrollNode, hasDatabase, registryOptions, testCa } from '../../control/test/nodes.ts'
import { publicContext } from '../../control/test/public-context.ts'

describe.skipIf(!hasDatabase)('provider CLI', () => {
  test('shapes/acquire/acquisition use one keyed fake acquisition', async () => {
    const c = await publicContext(),
      shape: Shape = {
        name: 'test',
        cpus: 4,
        memoryGb: 8,
        diskGb: 40,
        price: { currency: 'TEST', hour: '0', month: '0' },
      },
      provider = new FakeProvider([shape]),
      token = 'operator-provider-command-test-123456'
    const hosts = new ProviderLedger(
      c.db,
      c.boxes,
      new Map([['fake', provider]]),
      { url: 'https://127.0.0.1:8443', deployment: 'test', caPem: 'test-public-ca' },
      'stable-test-provider-seal-key',
      ['alpine:'],
    )
    const scaling = new Autoscale(
      c.db,
      c.boxes,
      hosts,
      { enabled: false, regions: {}, memoryFloorMb: 0, idleSeconds: 60, minNodes: 0, cpuMillisPerGb: 250 },
      THRESHOLDS,
    )
    const app = createOperatorApi({
      db: c.db,
      runtime: c.boxes.engine,
      thresholds: THRESHOLDS,
      token,
      relocate: async () => {},
    })
    withProviders(app, hosts, scaling)
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: app.fetch })
    const invoke = async (args: string[]) => {
      const child = Bun.spawn(['bun', 'cli/src/main.ts', 'node', ...args], {
        env: {
          ...process.env,
          PRYX_OPERATOR_API: server.url.toString().replace(/\/$/, ''),
          PRYX_OPERATOR_TOKEN: token,
        },
        stdout: 'pipe',
        stderr: 'pipe',
      })
      const out = await new Response(child.stdout).text(),
        err = await new Response(child.stderr).text()
      expect({ exit: await child.exited, err }).toEqual({ exit: 0, err: '' })
      return JSON.parse(out)
    }
    try {
      expect((await invoke(['shapes', 'default', '--provider', 'fake'])).shapes).toHaveLength(1)
      const first = await invoke([
        'acquire',
        'default',
        '--provider',
        'fake',
        '--shape',
        'test',
        '--idempotency-key',
        'cli-acquire',
      ])
      await hosts.settle()
      const repeated = await invoke([
        'acquire',
        'default',
        '--provider',
        'fake',
        '--shape',
        'test',
        '--idempotency-key',
        'cli-acquire',
      ])
      await hosts.settle()
      expect(repeated.id).toBe(first.id)
      expect(provider.creates).toBe(1)
      expect((await invoke(['acquisition', first.id])).state).toBe('waiting')
      const record = await hosts.record(first.id)
      const encoded = /sh -s -- (bk1\.[^ ]+)/.exec(hosts.bootstrap(record.id).line)?.[1] as string
      const secret = (
        JSON.parse(Buffer.from(encoded.slice('bk1.'.length), 'base64url').toString()) as { s: string }
      ).s
      const node = await enrollNode(c.db, await testCa(), 'cli-rental', { token: secret })
      await heartbeat(c.db, registryOptions(), node.id, node.sha, beat(node, 'cli-rental', 1))
      expect((await invoke(['release', node.id])).state).toBe('released')
    } finally {
      await hosts.settle()
      server.stop(true)
      await c.close()
    }
  })
})
