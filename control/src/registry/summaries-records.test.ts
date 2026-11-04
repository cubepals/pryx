// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Registry summaries preserve placement fields and audit health transitions once. */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  beat,
  capacity,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  testCa,
} from '../../../testing/fleet.ts'
import type { FleetCa } from '../ca.ts'
import { THRESHOLDS } from '../health.ts'
import { rows } from '../sql.ts'
import { heartbeat } from './heartbeat.ts'
import { getNode } from './node.ts'
import { holdsMemory, nodeSummaries, recordHealth, toNodeView } from './summaries.ts'

let db: Db
let ca: FleetCa
let drop: (() => Promise<void>) | undefined
beforeAll(async () => {
  if (!hasDatabase) return
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
})
afterAll(async () => {
  await drop?.()
})

test.skipIf(!hasDatabase)(
  'placement views retain every capacity field and default absent fields',
  async () => {
    const node = await enrollNode(db, ca, 'view-records')
    const available = capacity({ portsAllocated: 3 })
    await heartbeat(
      db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, 'session', 1, [], { capacity: available }),
    )
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((row) => row.id === node.id)
    if (!summary) throw new Error('The enrolled node summary must exist')
    expect(summary.allocated).toEqual({ memoryMb: 0, cpuMillis: 0, diskGb: 0, workloads: 0 })
    expect(summary.running).toEqual({ memoryMb: 0, cpuMillis: 0, workloads: 0 })
    expect(toNodeView(summary)).toEqual({
      id: node.id,
      name: summary.name,
      regionKey: summary.region_key,
      lifecycle: summary.lifecycle,
      health: summary.derivedHealth,
      quarantined: false,
      features: summary.features,
      capacity: {
        allocatableMemoryMb: available.allocatableMemoryMb,
        cpus: available.cpus,
        reservedCpuMillis: available.reservedCpuMillis,
        diskTotalBytes: available.diskTotalBytes,
        diskAvailableBytes: available.diskAvailableBytes,
        minFreeDiskMb: available.minFreeDiskMb,
        portsTotal: available.portsTotal,
        portsAllocated: available.portsAllocated,
      },
      allocated: summary.allocated,
      running: summary.running,
    })
    expect(
      toNodeView({ ...summary, capacity: {}, duplicate_seen_at: new Date().toISOString() }),
    ).toMatchObject({
      quarantined: true,
      capacity: {
        allocatableMemoryMb: 0,
        cpus: 0,
        reservedCpuMillis: 0,
        diskTotalBytes: null,
        diskAvailableBytes: null,
        minFreeDiskMb: 0,
        portsTotal: 0,
        portsAllocated: 0,
      },
    })
    expect(await holdsMemory(db, 'no-such-placement')).toBe(false)
    await db.execute(sql`UPDATE fleet_nodes SET lifecycle = 'retired' WHERE id = ${node.id}`)
    expect((await nodeSummaries(db, THRESHOLDS)).some((row) => row.id === node.id)).toBe(false)
    expect(
      (await nodeSummaries(db, THRESHOLDS, { includeRetired: true })).some((row) => row.id === node.id),
    ).toBe(true)
  },
)

test.skipIf(!hasDatabase)(
  'health changes are persisted and audited once with their measured age',
  async () => {
    const node = await enrollNode(db, ca, 'health-records')
    await heartbeat(db, registryOptions(), node.id, node.sha, beat(node, 'session', 1))
    const first = (await recordHealth(db, THRESHOLDS)).find((change) => change.node.id === node.id)
    expect(first).toMatchObject({ from: 'unavailable', to: 'healthy' })
    expect((await getNode(db, node.id))?.health).toBe('healthy')
    await db.execute(
      sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - make_interval(secs => ${THRESHOLDS.suspectSeconds + 1.25}) WHERE id = ${node.id}`,
    )
    const changed = (await recordHealth(db, THRESHOLDS)).find((change) => change.node.id === node.id)
    if (!changed || changed.node.age === null) throw new Error('The stale heartbeat must change health')
    expect(changed).toMatchObject({ from: 'healthy', to: 'suspect' })
    expect((await getNode(db, node.id))?.health).toBe('suspect')
    const events = () =>
      rows<{ data: unknown }>(
        db,
        sql`SELECT data FROM fleet_events WHERE node_id = ${node.id} AND kind = 'node.health' ORDER BY id`,
      )
    const recorded = await events()
    expect(recorded).toEqual([
      { data: { from: 'unavailable', to: 'healthy', ageSeconds: Number((first?.node.age ?? 0).toFixed(1)) } },
      { data: { from: 'healthy', to: 'suspect', ageSeconds: Number(changed.node.age.toFixed(1)) } },
    ])
    expect((await recordHealth(db, THRESHOLDS)).some((change) => change.node.id === node.id)).toBe(false)
    expect(await events()).toEqual(recorded)
  },
)

test.skipIf(!hasDatabase)(
  'a node that never beat records an unknown age and retired nodes stay untouched',
  async () => {
    const node = await enrollNode(db, ca, 'no-beat-records')
    await db.execute(sql`UPDATE fleet_nodes SET health = 'healthy' WHERE id = ${node.id}`)
    expect((await recordHealth(db, THRESHOLDS)).find((change) => change.node.id === node.id)).toMatchObject({
      from: 'healthy',
      to: 'unavailable',
    })
    expect(
      await rows(db, sql`SELECT data FROM fleet_events WHERE node_id = ${node.id} AND kind = 'node.health'`),
    ).toEqual([{ data: { from: 'healthy', to: 'unavailable', ageSeconds: null } }])
    await db.execute(
      sql`UPDATE fleet_nodes SET lifecycle = 'retired', health = 'healthy' WHERE id = ${node.id}`,
    )
    expect((await recordHealth(db, THRESHOLDS)).some((change) => change.node.id === node.id)).toBe(false)
    expect((await getNode(db, node.id))?.health).toBe('healthy')
  },
)
