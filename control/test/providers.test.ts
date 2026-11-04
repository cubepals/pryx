// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Provider identities, token trust and safe scaling use a real Postgres ledger and a fake rental backend. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { Autoscale, smallestShape } from '../src/autoscale.ts'
import { THRESHOLDS } from '../src/health.ts'
import { ProviderLedger } from '../src/provider-ledger.ts'
import { FakeProvider } from '../src/providers/fake.ts'
import type { Shape } from '../src/providers/provider.ts'
import { heartbeat } from '../src/registry.ts'
import { one } from '../src/sql.ts'
import { beat, enrollNode, hasDatabase, registryOptions, testCa } from './nodes.ts'
import { publicContext } from './public-context.ts'

const shape: Shape = {
  name: 'fit',
  cpus: 4,
  memoryGb: 8,
  diskGb: 40,
  price: { currency: 'TEST', hour: '0', month: '0' },
}
const tooSmall: Shape = { ...shape, name: 'small', cpus: 1, memoryGb: 2, diskGb: 5 }
describe.skipIf(!hasDatabase)('provider control', () => {
  let c: Awaited<ReturnType<typeof publicContext>>,
    provider: FakeProvider,
    hosts: ProviderLedger,
    scaling: Autoscale
  beforeAll(async () => {
    c = await publicContext()
    provider = new FakeProvider([tooSmall, shape])
    hosts = new ProviderLedger(
      c.db,
      c.boxes,
      new Map([['fake', provider]]),
      { deployment: 'test', url: 'https://127.0.0.1:8443', caPem: 'fixture-public-ca' },
      'test-only-stable-encryption-key',
      ['alpine:'],
    )
    scaling = new Autoscale(
      c.db,
      c.boxes,
      hosts,
      {
        enabled: true,
        regions: { grow: 'fake' },
        memoryFloorMb: 0,
        idleSeconds: 60,
        minNodes: 0,
        cpuMillisPerGb: 250,
      },
      THRESHOLDS,
    )
  })
  afterAll(async () => {
    await hosts?.settle()
    await c?.close()
  })
  const joined = async (id: string) => {
    const record = await hosts.record(id)
    const line = hosts.bootstrap(record.id).line,
      token = /sh -s -- (bk1\.[^ ]+)/.exec(line)?.[1]
    if (!token) throw new Error('Missing join token')
    const raw = JSON.parse(Buffer.from(token.slice('bk1.'.length), 'base64url').toString()) as { s: string }
    const node = await enrollNode(c.db, await testCa(), `rented-${id}`, {
      token: raw.s,
      region: record.region,
    })
    await heartbeat(
      c.db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, `session-${id}`, 1, [], {
        features: [
          'placement-epochs',
          'data-transfer',
          'local-snapshots',
          'export-exclude',
          'execution-lease',
        ],
      }),
    )
    await hosts.process(id)
    await c.boxes.engine.refreshRegistry()
    return node
  }
  test('smallest fit accounts for the daemon host reservations', () => {
    expect(smallestShape([shape, tooSmall], { memoryMb: 128, cpuMillis: 32, diskGb: 1 })?.name).toBe('fit')
  })
  test('lost acquire replies reconcile the same host and the ledger never contains the join token', async () => {
    provider.loseNextReply = true
    const first = await hosts.acquire('one', 'fake', 'fit', 'lost-reply')
    await hosts.settle()
    expect((await hosts.record(first.id)).state).toBe('uncertain')
    const before = provider.creates
    await hosts.acquire('one', 'fake', 'fit', 'lost-reply')
    await hosts.settle()
    const record = await hosts.record(first.id)
    expect(record.state).toBe('waiting')
    expect(provider.creates).toBe(before)
    expect(JSON.stringify(record)).not.toContain('bk1.')
    expect(JSON.stringify(await hosts.view(first.id))).not.toContain('bk1.')
    await expect(hosts.acquire('another', 'fake', 'fit', 'lost-reply')).rejects.toMatchObject({
      code: 'idempotency_key_reused',
    })
    const node = await joined(first.id)
    await scaling.tick()
    expect((await hosts.record(first.id)).node_id).toBe(node.id)
    expect((await hosts.record(first.id)).ready_at).not.toBeNull()
    expect((await scaling.release(node.id)).state).toBe('released')
    expect(provider.released.has(record.host?.id ?? '')).toBe(true)
  })
  test('claimed provider labels cannot bind a hand-enrolled node', async () => {
    const made = await hosts.acquire('spoof', 'fake', 'fit', 'spoof')
    await hosts.settle()
    const node = await enrollNode(c.db, await testCa(), 'spoof', {
      region: 'spoof',
      labels: { 'pryx.acquire': made.id },
    })
    await heartbeat(c.db, registryOptions(), node.id, node.sha, beat(node, 'spoof', 1))
    await hosts.process(made.id)
    expect((await hosts.record(made.id)).node_id).toBeNull()
  })
  test('growth requires two refusals, creates one pending host and resets after successful placement', async () => {
    const spec = {
      image: 'alpine:3.22',
      resources: { memoryMb: 128 },
      storage: { mountPath: '/data', sizeGb: 1 },
    }
    const before = provider.creates
    await scaling.refused('grow', spec)
    await scaling.tick()
    await hosts.settle()
    expect(provider.creates).toBe(before)
    await scaling.refused('grow', spec)
    await scaling.tick()
    await hosts.settle()
    expect(provider.creates).toBe(before + 1)
    await scaling.tick()
    await hosts.settle()
    expect(provider.creates).toBe(before + 1)
    await scaling.succeeded('grow')
    expect(
      (await one<{ refusals: number }>(c.db, sql`SELECT refusals FROM autoscale_regions WHERE region='grow'`))
        ?.refusals,
    ).toBe(0)
  })
  test('hand-joined, lost and quarantined nodes cannot be returned', async () => {
    const hand = (await c.boxes.list('default'))[0]
    expect(hand).toBeUndefined()
    await expect(scaling.release('00000000-0000-4000-8000-000000000000')).rejects.toMatchObject({
      code: 'refused',
    })
    const made = await hosts.acquire('unsafe', 'fake', 'fit', 'unsafe')
    await hosts.settle()
    const node = await joined(made.id)
    await c.db.execute(sql`UPDATE fleet_nodes SET duplicate_seen_at=now() WHERE id=${node.id}`)
    await expect(scaling.release(node.id)).rejects.toMatchObject({ code: 'refused' })
    await c.db.execute(
      sql`UPDATE fleet_nodes SET duplicate_seen_at=NULL,lifecycle='lost' WHERE id=${node.id}`,
    )
    await expect(scaling.release(node.id)).rejects.toMatchObject({ code: 'refused' })
    expect(provider.released.has((await hosts.record(made.id)).host?.id ?? '')).toBe(false)
  })
  test('an owned provider host is retained even when its node is idle', async () => {
    const owned = {
      ...provider,
      managed: false,
      name: 'owned',
      acquire: provider.acquire.bind(provider),
      bootstrap: provider.bootstrap.bind(provider),
      release: provider.release.bind(provider),
      list: provider.list.bind(provider),
      shapes: provider.shapes.bind(provider),
    }
    const registry = new ProviderLedger(
      c.db,
      c.boxes,
      new Map([['owned', owned]]),
      hosts.join,
      'test-stable-key',
      ['alpine:'],
    )
    const made = await registry.acquire('owned', 'owned', 'fit', 'owned')
    await registry.settle()
    const record = await registry.record(made.id)
    const node = await enrollNode(c.db, await testCa(), 'owned', { region: 'owned' })
    await c.db.execute(sql`UPDATE provider_acquisitions SET node_id=${node.id} WHERE id=${record.id}`)
    const control = new Autoscale(c.db, c.boxes, registry, scaling.config, THRESHOLDS)
    await expect(control.release(node.id)).rejects.toMatchObject({ code: 'refused' })
  })
  test('the memory floor grows one host and a region minimum blocks release', async () => {
    const floor = new Autoscale(
      c.db,
      c.boxes,
      hosts,
      { ...scaling.config, regions: { floor: 'fake' }, memoryFloorMb: 128, minNodes: 1 },
      THRESHOLDS,
    )
    const before = provider.creates
    await floor.tick()
    await hosts.settle()
    expect(provider.creates).toBe(before + 1)
    const a = await one<{ id: string }>(c.db, sql`SELECT id FROM provider_acquisitions WHERE region='floor'`)
    const node = await joined(a?.id ?? '')
    await expect(floor.release(node.id)).rejects.toMatchObject({ code: 'refused' })
  })
  test('release resumes after retirement when the vendor deletion reply was lost', async () => {
    const a = await hosts.acquire('release-retry', 'fake', 'fit', 'release-retry')
    await hosts.settle()
    const node = await joined(a.id)
    provider.loseNextReleaseReply = true
    await expect(scaling.release(node.id)).rejects.toThrow('reply lost')
    expect((await hosts.record(a.id)).state).toBe('releasing')
    await scaling.tick()
    expect((await hosts.record(a.id)).state).toBe('released')
  })
  test('every local snapshot reaches the store before idle rest releases a host', async () => {
    const a = await hosts.acquire('snapshots', 'fake', 'fit', 'snapshots')
    await hosts.settle()
    const node = await joined(a.id)
    const box = await c.boxes.make(
      'default',
      { image: 'alpine:3.22', resources: { memoryMb: 128 }, storage: { mountPath: '/data', sizeGb: 1 } },
      { region: 'snapshots' },
    )
    let failing = true
    const match = /POST .*\/snapshots\/.*\/upload$/
    const upload = async (answer: () => unknown): Promise<unknown> => {
      if (failing) {
        c.nodes.once(match, upload)
        throw new Error('store unavailable')
      }
      return answer()
    }
    c.nodes.once(match, upload)
    const older = await c.boxes.snapshot(box.id)
    await c.boxes.sleep(box.id)
    await expect(scaling.release(node.id)).rejects.toMatchObject({ code: 'not_stopped' })
    expect(provider.released.has((await hosts.record(a.id)).host?.id ?? '')).toBe(false)
    expect((await c.boxes.get(box.id)).node).toBe(node.id)
    failing = false
    await c.db.execute(
      sql`UPDATE fleet_archives SET status='local',attempts=0,upload_started_at=NULL WHERE workload=${box.id} AND status='failed'`,
    )
    expect((await scaling.release(node.id)).state).toBe('released')
    expect(
      (await one<{ status: string }>(c.db, sql`SELECT status FROM fleet_archives WHERE id=${older.id}`))
        ?.status,
    ).toBe('ready')
    expect((await c.boxes.get(box.id)).state).toBe('resting')
    await c.boxes.delete(box.id)
  })
  test('an unjoined acquisition can be cancelled without ever minting a second host', async () => {
    const a = await hosts.acquire('cancel', 'fake', 'fit', 'cancel')
    await hosts.settle()
    const count = provider.creates
    expect((await hosts.cancel(a.id)).state).toBe('released')
    await hosts.acquire('cancel', 'fake', 'fit', 'cancel')
    await hosts.settle()
    expect(provider.creates).toBe(count)
  })
  test('queued lock waiters leave a connection available for nested intents', async () => {
    let n = 0
    await Promise.all(
      Array.from({ length: 20 }, () =>
        c.boxes.exclusive('outer-test', () =>
          c.boxes.exclusive('inner-test', async () => {
            n++
          }),
        ),
      ),
    )
    expect(n).toBe(20)
  })
  test('idle expiry returns an empty managed node', async () => {
    const a = await hosts.acquire('idle', 'fake', 'fit', 'idle')
    await hosts.settle()
    const node = await joined(a.id)
    await scaling.tick()
    await c.db.execute(
      sql`UPDATE provider_node_state SET idle_since=now()-interval '2 minutes' WHERE node_id=${node.id}`,
    )
    await scaling.tick()
    expect((await hosts.record(a.id)).state).toBe('released')
  })
})
