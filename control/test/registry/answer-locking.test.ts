// SPDX-License-Identifier: FSL-1.1-ALv2
/** A first power answer and a heartbeat take the node lock before its observation locks. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import type { Db } from '../../src/database.ts'
import { SECOND_MS } from '../../src/durations.ts'
import { answered } from '../../src/registry.ts'
import { jsonb, one, rows } from '../../src/sql.ts'
import type { WorkloadView } from '../../src/wire.ts'
import { enrollNode, freshDatabase, hasDatabase, report, testCa } from '../nodes.ts'

let db: Db
let drop: (() => Promise<void>) | undefined
beforeAll(async () => {
  if (hasDatabase) ({ db, drop } = await freshDatabase())
})
afterAll(async () => {
  await drop?.()
})

describe.skipIf(!hasDatabase)('power answer lock ordering', () => {
  test('a first answer waits for the node before reserving an observation', async () => {
    const node = await enrollNode(db, await testCa(), 'answer-lock-order')
    const workload = 'first-answer-lock'
    const old = report(workload, 7, { state: 'stopped', changedAt: '2026-01-01T00:00:00Z' })
    const view: WorkloadView = {
      ...old,
      state: 'running',
      changedAt: '2026-01-01T00:00:01Z',
      guarantees: undefined,
      image: 'alpine:3.22',
      ports: [],
      startedAt: null,
      finishedAt: null,
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
    await db.execute(sql`INSERT INTO fleet_placements
      (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb, desired_power)
      VALUES (${workload}, ${node.id}, 7, 'placed', 'eu', 128, 32, 1, 'running')`)
    const connected = Promise.withResolvers<number>()
    let answering: Promise<{ value: boolean } | { error: unknown }> | undefined
    let result: { value: boolean } | { error: unknown } | undefined
    try {
      await db.transaction(async (beat) => {
        // The heartbeat already owns this node while preparing its whole observation batch.
        await beat.execute(sql`SELECT id FROM fleet_nodes WHERE id = ${node.id} FOR UPDATE`)
        answering = db
          .transaction(async (answer) => {
            const pid = await one<{ pid: number }>(answer, sql`SELECT pg_backend_pid() AS pid`)
            if (!pid) throw new Error('Missing answer connection')
            connected.resolve(pid.pid)
            return answered(answer, node.id, view)
          })
          .then(
            (value) => ({ value }),
            (error: unknown) => ({ error }),
          )
        const pid = await connected.promise
        const deadline = Date.now() + SECOND_MS
        // PostgreSQL 17 monitoring-stats: wait_event_type 'Lock' means a heavyweight lock wait.
        for (;;) {
          const blocked = await one<{ waiting: boolean }>(
            db,
            sql`SELECT wait_event_type = 'Lock' AS waiting FROM pg_stat_activity WHERE pid = ${pid}`,
          )
          if (blocked?.waiting) break
          if (Date.now() >= deadline) throw new Error('Answer never waited for the held node')
          await Bun.sleep(SECOND_MS / 1000)
        }
        // As in heartbeat(), insert the first report while the node lock remains held.
        // An answer reserving this observation before its foreign-key node lock deadlocks here.
        await beat.execute(sql`INSERT INTO fleet_observations
          (node_id, workload, epoch, state, spec_digest, report)
          VALUES (${node.id}, ${workload}, 7, 'stopped', ${old.specDigest}, ${jsonb(old)})`)
      })
    } finally {
      result = await answering
    }
    if (result && 'error' in result) throw result.error
    expect(result).toEqual({ value: true })
    expect(await rows(db, sql`SELECT state FROM fleet_observations WHERE workload = ${workload}`)).toEqual([
      { state: 'running' },
    ])
  })
})
