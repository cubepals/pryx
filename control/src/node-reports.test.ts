// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What a node's heartbeat may change, and what it may not: a node speaks for its own copies only,
 * a second host with its identity is given nothing to act on, and the ledger takes a copy as made
 * only once its node has made it.
 */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  beat,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  report,
  testCa,
} from '../../testing/fleet.ts'
import type { FleetCa } from './ca.ts'
import { Ledger, nextEpoch } from './fleet-runtime/ledger.ts'
import { THRESHOLDS } from './health.ts'
import { heartbeat, nodeSummaries } from './registry.ts'
import { rows } from './sql.ts'
import type { HeartbeatRequest } from './wire.ts'

let db: Db
let ca: FleetCa
let drop: () => Promise<void>

const send = (n: { id: string; sha: string }, request: HeartbeatRequest) =>
  heartbeat(db, registryOptions(), n.id, n.sha, request)

const summary = async (id: string) => (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === id)

/** The session's switch, `seconds` ago. */
const switchedAgo = (id: string, seconds: number) =>
  db.execute(
    sql`UPDATE fleet_nodes SET session_started_at = now() - make_interval(secs => ${seconds}) WHERE id = ${id}`,
  )
/** Past the previous session's in-flight window, and not yet past the new session's hold. */
const pastInFlight = (id: string) => switchedAgo(id, 20)
/** Past the new session's hold: the previous session's beat in flight, a heartbeat, and a margin. */
const pastHold = (id: string) => switchedAgo(id, 30)

async function place(workload: string, nodeId: string, epoch: number, state = 'placed'): Promise<void> {
  await db.execute(sql`
    INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb,
      desired_power, power_changed_at)
    VALUES (${workload}, ${nodeId}, ${epoch}, ${state}, 'eu', 1024, 250, 2, 'running', now())`)
  await db.execute(sql`
    INSERT INTO fleet_placement_history (workload, epoch, node_id, reason)
    VALUES (${workload}, ${epoch}, ${nodeId}, 'placed')`)
}

beforeAll(async () => {
  if (!hasDatabase) return
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
  await db.execute(sql`INSERT INTO fleet_endpoints VALUES ('test', now() - interval '1 hour', now())`)
})
afterAll(() => drop?.())

describe.skipIf(!hasDatabase)('the lease, and who may beat for a node', () => {
  test('a second host beating with a new session gets no lease and no edge, then or once caught', async () => {
    const a = await enrollNode(db, ca, 'clone-lease', { apiAddress: '10.0.0.5:7443' })
    for (let seq = 1; seq <= 3; seq++) await send(a, beat(a, 'original', seq))
    const elsewhere = { addresses: { api: '10.0.0.9:7443', edge: '10.0.0.9', control: '10.0.0.9' } }
    // A copy of the disk boots: a new session while the original still beats.
    expect((await send(a, beat(a, 'clone', 1, [], elsewhere))).leaseSeconds).toBe(0)
    expect((await send(a, beat(a, 'clone', 2, [], elsewhere))).leaseSeconds).toBe(0)
    expect((await summary(a.id))?.edge_host).toBe('10.0.0.5')
    // The original beats on past the in-flight window, and is caught.
    await pastInFlight(a.id)
    await expect(send(a, beat(a, 'original', 4))).rejects.toMatchObject({ code: 'duplicate_identity' })
    // Quarantined: nothing resumes on its own until an operator decides, and the edge stays put.
    expect((await send(a, beat(a, 'clone', 3, [], elsewhere))).leaseSeconds).toBe(0)
    expect((await summary(a.id))?.edge_host).toBe('10.0.0.5')
  })

  test("a second host's beats leave the copies the node reported, and their room, as they were", async () => {
    const a = await enrollNode(db, ca, 'clone-copies')
    await place('w-kept-1', a.id, 1)
    await place('w-kept-2', a.id, 1)
    const copies = [report('w-kept-1', 1), report('w-kept-2', 1)]
    for (let seq = 1; seq <= 3; seq++) await send(a, beat(a, 'original', seq, copies))
    const observed = () =>
      rows<{ workload: string; state: string }>(
        db,
        sql`SELECT workload, state FROM fleet_observations WHERE node_id = ${a.id} ORDER BY workload`,
      )
    const before = await observed()
    expect(before).toHaveLength(2)
    expect((await summary(a.id))?.running.memoryMb).toBe(2048)
    await send(a, beat(a, 'clone', 1))
    await send(a, beat(a, 'clone', 2, [report('w-kept-1', 1, { state: 'stopped' })]))
    expect(await observed()).toEqual(before)
    expect((await summary(a.id))?.running.memoryMb).toBe(2048)
    // Caught and quarantined, it changes nothing either.
    await pastInFlight(a.id)
    await expect(send(a, beat(a, 'original', 4, copies))).rejects.toMatchObject({
      code: 'duplicate_identity',
    })
    await send(a, beat(a, 'clone', 3))
    expect(await observed()).toEqual(before)
    expect((await summary(a.id))?.running.memoryMb).toBe(2048)
  })

  test("a second host's beat just past the in-flight window, before the original's next, is still held", async () => {
    const a = await enrollNode(db, ca, 'clone-edge')
    await place('w-edge', a.id, 1)
    const copies = [report('w-edge', 1)]
    for (let seq = 1; seq <= 3; seq++) await send(a, beat(a, 'original', seq, copies))
    const observed = () =>
      rows<{ workload: string; state: string }>(
        db,
        sql`SELECT workload, state FROM fleet_observations WHERE node_id = ${a.id}`,
      )
    const before = await observed()
    expect((await send(a, beat(a, 'clone', 1))).leaseSeconds).toBe(0)
    // Both beat every few seconds: the clone's comes first after the window.
    await switchedAgo(a.id, 16)
    expect((await send(a, beat(a, 'clone', 2))).leaseSeconds).toBe(0)
    expect(await observed()).toEqual(before)
    // The original's next beat, a heartbeat later and a little late, catches the pair.
    await switchedAgo(a.id, 16 + registryOptions().heartbeatSeconds + 3)
    await expect(send(a, beat(a, 'original', 4, copies))).rejects.toMatchObject({
      code: 'duplicate_identity',
    })
    expect((await summary(a.id))?.duplicate_seen_at).not.toBeNull()
    expect((await send(a, beat(a, 'clone', 3))).leaseSeconds).toBe(0)
    expect(await observed()).toEqual(before)
  })

  test('a rebooted node gets its lease, and is reached at its new address, once the old session is past', async () => {
    const a = await enrollNode(db, ca, 'reboot-lease', { apiAddress: '10.0.0.6:7443' })
    expect((await send(a, beat(a, 'one', 1))).leaseSeconds).toBe(120)
    const rebooted = {
      bootId: 'boot-2',
      addresses: { api: '10.0.0.7:7443', edge: '10.0.0.7', control: '10.0.0.7' },
    }
    expect((await send(a, beat(a, 'two', 1, [], rebooted))).leaseSeconds).toBe(0)
    // Past the in-flight window, it is still held: the old session's next beat may yet come.
    await switchedAgo(a.id, 16)
    expect((await send(a, beat(a, 'two', 2, [], rebooted))).leaseSeconds).toBe(0)
    await pastHold(a.id)
    expect((await send(a, beat(a, 'two', 3, [], rebooted))).leaseSeconds).toBe(120)
    expect((await summary(a.id))?.edge_host).toBe('10.0.0.7')
  })
})

describe.skipIf(!hasDatabase)('epochs, as nodes report them', () => {
  test('an epoch past the exact integers is refused', async () => {
    const a = await enrollNode(db, ca, 'epoch-exact')
    await expect(send(a, beat(a, 's', 1, [report('w-exact', 2 ** 53)]))).rejects.toMatchObject({
      code: 'invalid_request',
    })
    await expect(
      send(a, beat(a, 's', 1, [report('w-exact', 1, { supersededBy: 2 ** 53 })])),
    ).rejects.toMatchObject({ code: 'invalid_request' })
  })

  test('one up to the ceiling is taken, and the epochs after it stay exact', async () => {
    const a = await enrollNode(db, ca, 'epoch-ceiling')
    await expect(send(a, beat(a, 's', 1, [report('w-high', 2 ** 52 + 1)]))).rejects.toMatchObject({
      code: 'invalid_request',
    })
    await send(a, beat(a, 's', 1, [report('w-high', 2 ** 52)]))
    const next = await nextEpoch(db, 'w-high')
    expect(next).toBe(2 ** 52 + 1)
    expect(Number.isSafeInteger(next)).toBe(true)
  })

  test("every node's copy counts, so a restored database never hands out an epoch a node holds", async () => {
    const a = await enrollNode(db, ca, 'restored-a')
    const b = await enrollNode(db, ca, 'restored-b')
    // The backup has the server on A at 3; since then it moved to B at 4.
    await place('w-restored', a.id, 3)
    await send(b, beat(b, 's', 1, [report('w-restored', 4)]))
    expect(await nextEpoch(db, 'w-restored')).toBe(5)
    // Given a new home at that epoch, B's copy is fenced.
    await db.execute(sql`UPDATE fleet_placements SET epoch = 5 WHERE workload = 'w-restored'`)
    expect((await send(b, beat(b, 's', 2, [report('w-restored', 4)]))).fences).toEqual([
      { workload: 'w-restored', currentEpoch: 5 },
    ])
  })
})

describe.skipIf(!hasDatabase)('a first placement, as its node reports it', () => {
  test('one whose copy is still being made waits for it, and is undone if it never is', async () => {
    const ledger = new Ledger({ db, deployment: 'test' })
    const a = await enrollNode(db, ca, 'pulling')
    await place('w-pull', a.id, 1, 'placing')
    await db.execute(
      sql`UPDATE fleet_placements SET updated_at = now() - interval '31 minutes' WHERE workload = 'w-pull'`,
    )
    // The node took the PUT and is pulling the image.
    await send(a, beat(a, 's', 1, [report('w-pull', 1, { state: 'creating', ports: {} })]))
    expect(await ledger.finishReported()).toBe(0)
    expect(await ledger.abandonUnmade()).toBe(0)
    const state = async () =>
      (
        await rows<{ state: string }>(db, sql`SELECT state FROM fleet_placements WHERE workload = 'w-pull'`)
      )[0]?.state
    expect(await state()).toBe('placing')
    // The pull fails, and the node forgets the copy: the placement goes, and its room with it.
    await send(a, beat(a, 's', 2, []))
    expect(await ledger.abandonUnmade()).toBe(1)
    expect(await state()).toBeUndefined()
    expect((await summary(a.id))?.running.memoryMb).toBe(0)
  })

  test('one whose copy is made is finished from its report', async () => {
    const ledger = new Ledger({ db, deployment: 'test' })
    const a = await enrollNode(db, ca, 'made')
    await place('w-made', a.id, 1, 'placing')
    await db.execute(
      sql`UPDATE fleet_placements SET updated_at = now() - interval '1 minute' WHERE workload = 'w-made'`,
    )
    await send(a, beat(a, 's', 1, [report('w-made', 1, { state: 'created' })]))
    expect(await ledger.finishReported()).toBe(1)
  })
})
