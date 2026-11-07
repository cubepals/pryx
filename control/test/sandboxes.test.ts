// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Sandbox intent and locks over real Postgres with the fleet's node double; no Docker is used. */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { FleetStore } from '../src/archive-transport.ts'
import { createPool } from '../src/database.ts'
import { THRESHOLDS } from '../src/health.ts'
import type { NodeClient } from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { heartbeat } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import { Sandboxes, type SandboxSpec } from '../src/sandboxes.ts'
import { FlakyStore, Nodes } from './flow-nodes.ts'
import { beat, enrollNode, freshDatabase, hasDatabase, registryOptions, testCa } from './nodes.ts'

const spec: SandboxSpec = {
  image: 'alpine:3.22',
  resources: { memoryMb: 1024 },
  storage: { mountPath: '/data', sizeGb: 1 },
  ports: [
    {
      name: 'http',
      containerPort: 8080,
      protocol: 'tcp',
      audience: ['edge', 'control'],
    },
  ],
  secrets: { TOKEN: 'keep-this-private' },
}
describe.skipIf(!hasDatabase)('sandbox intent on Postgres', () => {
  let service: Sandboxes
  let nodes: Nodes
  let store: FlakyStore
  let close: () => Promise<void>
  beforeAll(async () => {
    const fresh = await freshDatabase()
    const locks = createPool(fresh.url)
    store = new FlakyStore()
    await store.start()
    nodes = new Nodes(store)
    const client = nodes as unknown as NodeClient
    const engine = new FleetRuntime({
      db: fresh.db,
      nodes: client,
      store: new FleetStore(store, 'test'),
      deployment: 'test',
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
    })
    const ca = await testCa()
    const node = await enrollNode(fresh.db, ca, 'sandbox-node', {
      region: 'default',
    })
    await heartbeat(
      fresh.db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, 'session', 1, [], {
        features: [
          'placement-epochs',
          'data-transfer',
          'local-snapshots',
          'export-exclude',
          'execution-lease',
        ],
      }),
    )
    await engine.refreshRegistry()
    service = new Sandboxes(fresh.db, engine, locks, client)
    close = async () => {
      await locks.end()
      await store.close()
      await fresh.drop()
    }
  })
  afterAll(async () => {
    await close?.()
  })

  test('start false makes storage without starting application code', async () => {
    const since = nodes.calls.length
    const made = await service.make('project', spec, { start: false })
    expect(made.state).toBe('stopped')
    expect(nodes.calls.slice(since).some((call) => call.path.endsWith('/start'))).toBe(false)
    expect(JSON.stringify(made)).not.toContain('keep-this-private')
    await service.delete(made.id, 'project')
  })
  test('a retried make reads a JSONB spec independent of object key order', async () => {
    const id = crypto.randomUUID()
    const first = await service.make('project', spec, { id })
    const again = await service.make(
      'project',
      { ...spec, storage: { sizeGb: 1, mountPath: '/data' } },
      { id },
    )
    expect(again.id).toBe(first.id)
    await expect(service.get(id, 'another-project')).rejects.toMatchObject({
      code: 'not_found',
    })
    await service.delete(id, 'project')
  })
  test('sleep and wake report their tier and keep the epoch', async () => {
    const made = await service.make('project', spec)
    const asleep = await service.sleep(made.id, 'project')
    expect(asleep).toMatchObject({
      tier: 'stopped',
      sandbox: { state: 'sleeping' },
    })
    expect(await service.wake(made.id, 'project')).toMatchObject({
      tier: 'booted',
      sandbox: { state: 'running', epoch: made.epoch },
    })
    await service.delete(made.id, 'project')
  })
  test('snapshot, fork, rest and restore keep the uploaded source', async () => {
    const made = await service.make('project', spec)
    const snap = await service.snapshot(made.id, 'project')
    const fork = await service.make('project', spec, { snapshot: snap.id })
    expect(fork.id).not.toBe(made.id)
    expect(
      nodes.calls.findLast((call) => call.path === `/v1/workloads/${fork.id}/restore`)?.body,
    ).toHaveProperty('url')
    expect((await service.rest(made.id, 'project')).state).toBe('resting')
    expect(await service.wake(made.id, 'project')).toMatchObject({
      tier: 'restored',
      sandbox: { state: 'running' },
    })
    await service.delete(fork.id, 'project')
    await service.delete(made.id, 'project')
  })
  test('upkeep deletes elapsed TTL and sleeps idle sandboxes', async () => {
    const ttl = await service.make('project', { ...spec, ttl: 1 })
    await service.db.execute(
      sql`UPDATE sandboxes SET expires_at = now() - interval '1 second' WHERE id = ${ttl.id}`,
    )
    const idle = await service.make('project', { ...spec, idle: 1 })
    await service.db.execute(
      sql`UPDATE sandboxes SET activity_at = now() - interval '5 minutes' WHERE id = ${idle.id}`,
    )
    await expect(service.get(ttl.id, 'project')).rejects.toMatchObject({ code: 'gone' })
    await service.upkeep()
    await expect(service.get(ttl.id, 'project')).rejects.toMatchObject({
      code: 'gone',
    })
    expect((await service.get(idle.id, 'project')).state).toBe('sleeping')
    await service.delete(idle.id, 'project')
  })
  test('upkeep finishes a stopped creation after its final state write was interrupted', async () => {
    const made = await service.make('project', spec, { start: false })
    await service.db.execute(sql`UPDATE sandboxes SET state = 'creating' WHERE id = ${made.id}`)
    const since = nodes.calls.length
    await service.upkeep()
    expect((await service.get(made.id, 'project')).state).toBe('stopped')
    expect(nodes.calls.slice(since).some((call) => call.path.endsWith('/start'))).toBe(false)
    await service.delete(made.id, 'project')
  })
  test('placement reserves the CPU request sent to the node', async () => {
    const made = await service.make('project', { ...spec, resources: { memoryMb: 1024, cpuMillis: 1750 } })
    const result = await service.db.execute(
      sql`SELECT cpu_millis FROM fleet_placements WHERE workload = ${made.id}`,
    )
    expect(result.rows[0]).toMatchObject({ cpu_millis: 1750 })
    await service.delete(made.id, 'project')
  })
})
