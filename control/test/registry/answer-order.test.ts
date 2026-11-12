// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Delayed heartbeats cannot replace a newer power response with an older workload state. */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import type { FleetCa } from '../../src/ca.ts'
import type { Db } from '../../src/database.ts'
import { answered, heartbeat, holdsMemory } from '../../src/registry.ts'
import { rows } from '../../src/sql.ts'
import type { WorkloadView } from '../../src/wire.ts'
import { beat, enrollNode, freshDatabase, hasDatabase, registryOptions, report, testCa } from '../nodes.ts'

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

const earlier = '2026-01-01T00:00:00Z'
const later = '2026-01-01T00:00:01Z'

describe.skipIf(!hasDatabase)('ordering a node’s reports of its own copy', () => {
  test('a complete first answer establishes only its assigned node and epoch', async () => {
    const node = await enrollNode(db, ca, 'first-answer')
    const other = await enrollNode(db, ca, 'wrong-answer')
    const workload = 'first-answered-copy'
    await db.execute(sql`INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb, desired_power)
      VALUES (${workload}, ${node.id}, 7, 'placed', 'eu', 128, 32, 1, 'running')`)
    const running: WorkloadView = {
      ...report(workload, 7, { state: 'running', changedAt: later }),
      guarantees: undefined,
      ports: [],
      image: 'alpine:3.22',
      startedAt: later,
      finishedAt: null,
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
    expect(await answered(db, other.id, running)).toBe(false)
    expect(await answered(db, node.id, { ...running, epoch: 8 })).toBe(false)
    expect(await answered(db, node.id, { ...running, id: 'unassigned-answer' })).toBe(false)
    expect(await rows(db, sql`SELECT workload FROM fleet_observations WHERE workload = ${workload}`)).toEqual(
      [],
    )
    expect(await answered(db, node.id, running)).toBe(true)
    expect(
      await rows<{ report: { state: string; memoryMb: number }; epoch: string }>(
        db,
        sql`SELECT report,epoch FROM fleet_observations WHERE workload = ${workload}`,
      ),
    ).toMatchObject([{ epoch: '7', report: { state: 'running', memoryMb: 128 } }])
    await heartbeat(
      db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, 'session', 1, [report(workload, 7, { state: 'stopped', changedAt: earlier })]),
    )
    expect(await holdsMemory(db, workload)).toBe(true)
    await heartbeat(
      db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, 'session', 2, [
        report(workload, 7, { state: 'stopped', changedAt: '2026-01-01T00:00:02Z' }),
      ]),
    )
    expect(await holdsMemory(db, workload)).toBe(false)
  })
  test('a heartbeat sampled before a stop answer keeps the released memory free', async () => {
    const node = await enrollNode(db, ca, 'stop-answer')
    const workload = 'stopped-copy'
    const send = (seq: number, state: 'running' | 'stopped', changedAt: string) =>
      heartbeat(
        db,
        registryOptions(),
        node.id,
        node.sha,
        beat(node, 'session', seq, [report(workload, 1, { state, changedAt })]),
      )
    await send(1, 'running', earlier)
    await db.execute(sql`INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb, desired_power)
      VALUES (${workload}, ${node.id}, 1, 'placed', 'eu', 128, 32, 1, 'stopped')`)
    const stopped: WorkloadView = {
      ...report(workload, 1, { state: 'stopped', changedAt: later }),
      guarantees: undefined,
      ports: [],
      image: 'alpine:3.22',
      startedAt: earlier,
      finishedAt: later,
      locate: {
        containerName: workload,
        containerId: null,
        dataDir: '/data',
      },
    }
    expect(await answered(db, node.id, stopped)).toBe(true)
    expect(await holdsMemory(db, workload)).toBe(false)
    // The heartbeat has a newer sequence but carries a state sampled before the stop.
    await send(2, 'running', earlier)
    expect(await holdsMemory(db, workload)).toBe(false)
    expect(
      await rows<{ state: string }>(
        db,
        sql`SELECT state FROM fleet_observations WHERE workload = ${workload}`,
      ),
    ).toEqual([{ state: 'stopped' }])
    // A genuine later restart is still authoritative.
    await send(3, 'running', '2026-01-01T00:00:02Z')
    expect(await holdsMemory(db, workload)).toBe(true)
    expect(await answered(db, node.id, stopped)).toBe(false)
    expect(await holdsMemory(db, workload)).toBe(true)
  })
})
