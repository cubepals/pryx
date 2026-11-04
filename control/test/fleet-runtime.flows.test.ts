// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The flows through FleetRuntime that matter most, pinned as they are: which node calls each makes,
 * in what order, with which epoch and timeout, and what each leaves in the ledger and the archives.
 * Its pryxd in memory is a smaller copy of fleet-runtime.test.ts's, which can't move to a file of
 * its own beside the adapter (an adapter file may not import test support) nor be imported from a
 * test file without running that file's tests.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { FleetStore } from '../src/archive-transport.ts'
import type { FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { decodeHandle, encodeSnapshot } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import { type NodeClient, NodeRefused, NodeUnreachable } from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { drain, heartbeat, nodeSummaries } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import {
  type ProgressSink,
  RuntimeFull,
  type RuntimeKey,
  type RuntimeSpec,
  runtimeKey,
  type SnapshotHandle,
} from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import { TEARDOWN_MS, TRANSFER_MS } from '../src/workload-api.ts'
import { FlakyStore, Nodes } from './flow-nodes.ts'
import {
  beat,
  capacity,
  type EnrolledNode,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  testCa,
} from './nodes.ts'

const SPEC: RuntimeSpec = {
  image: 'alpine:3.22',
  env: {},
  secrets: {},
  resources: { memoryMb: 1024 },
  storage: { mountPath: '/data', sizeGb: 1, reconstructible: ['libraries'] },
  ports: [
    {
      name: 'game',
      port: 8080,
      protocol: 'tcp',
      audience: ['edge', 'control'],
    },
  ],
  stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
  labels: {},
}

const FEATURES = [
  'placement-epochs',
  'data-transfer',
  'local-snapshots',
  'export-exclude',
  'execution-lease',
  'multipart-upload',
]
const CONFIRM = 'x-blockly-confirm-delete-data'

describe.skipIf(!hasDatabase)('FleetRuntime flows, with pryxd in memory', () => {
  let db: Db
  let drop: () => Promise<void>
  let ca: FleetCa
  let store: FlakyStore
  let nodes: Nodes
  let runtime: FleetRuntime
  const logs: Array<{ message: string; fields: object | undefined }> = []
  let seq = 0

  const pulse = (n: EnrolledNode) =>
    heartbeat(
      db,
      registryOptions(),
      n.id,
      n.sha,
      beat(n, 'session', ++seq, nodes.reports(n.id), {
        features: FEATURES,
        capacity: capacity({ allocatableMemoryMb: 4096 }),
      }),
    )

  /** Nodes of their own region, healthy, named `<name>-<i>`, so each test places only on its own. */
  async function region(name: string, count: number): Promise<EnrolledNode[]> {
    const made: EnrolledNode[] = []
    for (let i = 0; i < count; i++) {
      const n = await enrollNode(db, ca, `${name}-${i}`, { region: name })
      await pulse(n)
      made.push(n)
    }
    await runtime.refreshRegistry()
    return made
  }

  const server = (): RuntimeKey => runtimeKey(randomUUID())
  const nameOf = async (node: string) =>
    (await rows<{ name: string }>(db, sql`SELECT name FROM fleet_nodes WHERE id = ${node}`))[0]?.name ?? ''

  /** Every node call since `from`, as `<node name> <method> <path> e<epoch> t<timeout>`. */
  async function callsSince(from: number): Promise<string[]> {
    const lines: string[] = []
    for (const c of nodes.calls.slice(from))
      lines.push(`${await nameOf(c.node)} ${c.method} ${c.path} e${c.epoch ?? '-'} t${c.timeoutMs ?? '-'}`)
    return lines
  }
  const kinds = async (workload: string, after = 0) =>
    (
      await rows<{ kind: string }>(
        db,
        sql`SELECT kind FROM fleet_events WHERE workload = ${workload} AND id > ${after} ORDER BY id`,
      )
    ).map((e) => e.kind)
  const lastEvent = async () =>
    Number((await rows<{ id: string | null }>(db, sql`SELECT max(id) AS id FROM fleet_events`))[0]?.id ?? 0)
  const eventData = (kind: string, workload: string) =>
    rows<{
      node_id: string | null
      epoch: string | null
      data: Record<string, unknown>
    }>(
      db,
      sql`SELECT node_id, epoch, data FROM fleet_events WHERE kind = ${kind} AND workload = ${workload} ORDER BY id`,
    )
  const placement = async (workload: string) =>
    (
      await rows<{
        node_id: string | null
        epoch: string
        state: string
        desired_power: string
        completing: string | null
        restore_from: string | null
        move_requested_at: string | null
      }>(
        db,
        sql`SELECT node_id, epoch, state, desired_power, completing, restore_from, move_requested_at
            FROM fleet_placements WHERE workload = ${workload}`,
      )
    )[0]
  const history = (workload: string) =>
    rows<{
      epoch: string
      node_id: string | null
      reason: string
      end_reason: string | null
    }>(
      db,
      sql`SELECT epoch, node_id, reason, end_reason FROM fleet_placement_history WHERE workload = ${workload}
          ORDER BY epoch`,
    )
  const archives = (workload: string) =>
    rows<{
      id: string
      purpose: string
      status: string
      object_key: string | null
      local_id: string | null
      local_deleted_at: Date | null
      attempts: number
      error: string | null
    }>(
      db,
      sql`SELECT id, purpose, status, object_key, local_id, local_deleted_at, attempts, error
          FROM fleet_archives WHERE workload = ${workload} ORDER BY captured_at`,
    )
  const running = async (node: string) =>
    (await nodeSummaries(db, THRESHOLDS, { includeRetired: true })).find((n) => n.id === node)?.running
      .memoryMb
  const stored = (workload: string) => [...store.objects.keys()].filter((key) => key.includes(workload))

  /** What a call to ProgressSink was, in order. */
  function recorder(): { sink: ProgressSink; seen: string[] } {
    const seen: string[] = []
    return {
      seen,
      sink: {
        step: async (step) => {
          seen.push(step)
        },
        handle: async (handle) => {
          seen.push(`handle e${decodeHandle(handle).epoch}`)
        },
      },
    }
  }
  const quiet: ProgressSink = { step: async () => {}, handle: async () => {} }

  /** A snapshot its node holds a copy of; also in the store, ready, when `objectKey` is given. */
  async function snapshotOn(
    node: string,
    workload: string,
    objectKey: string | null,
    over: { attempts?: number; capturedAgo?: string } = {},
  ): Promise<{ id: string; handle: SnapshotHandle }> {
    const id = randomUUID()
    await db.execute(sql`
      INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, local_id, object_key, status, consistency,
                                  captured_at, attempts)
      VALUES (${id}, ${workload}, 'snapshot', 1, ${node}, ${id}, ${objectKey},
              ${objectKey === null ? 'local' : 'ready'}, 'stopped',
              now() - ${over.capturedAgo ?? '0 seconds'}::interval, ${over.attempts ?? 0})`)
    if (objectKey !== null) store.objects.set(objectKey, Buffer.from('data!'))
    return {
      id,
      handle: encodeSnapshot({
        deployment: 'test',
        key: workload,
        archive: id,
        epoch: 1,
      }),
    }
  }

  /** A server running on one of the region's two nodes, with a move to the other asked for. */
  async function moving(name: string) {
    const [a, b] = (await region(name, 2)) as [EnrolledNode, EnrolledNode]
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: name }, SPEC, quiet)
    const from = decodeHandle(handle).node === a.id ? a : b
    const to = from === a ? b : a
    await runtime.requestMove(key, to.id, 'operator:test')
    return { key, handle, from, to }
  }

  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    ca = await testCa()
    store = new FlakyStore()
    await store.start()
    nodes = new Nodes(store)
    runtime = new FleetRuntime({
      db,
      nodes: nodes as unknown as NodeClient,
      store: new FleetStore(store, 'test'),
      deployment: 'test',
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
      log: (message, fields) => {
        logs.push({ message, fields })
      },
    })
  })
  afterAll(async () => {
    store.close()
    await drop?.()
  })

  test('1. a first start on a node with room: PUT, then start, with its room claimed', async () => {
    const [n] = (await region('first', 1)) as [EnrolledNode]
    const key = server()
    const since = nodes.calls.length
    const { sink, seen } = recorder()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'first' }, SPEC, sink)
    expect(await callsSince(since)).toEqual([
      `first-0 PUT /v1/workloads/${key} e1 t1200000`,
      `first-0 POST /v1/workloads/${key}/start e1 t60000`,
    ])
    expect(nodes.calls[since]?.body).toMatchObject({
      image: 'alpine:3.22',
      restart: { policy: 'on-failure' },
    })
    expect(seen).toEqual(['allocating', 'compute', 'handle e1', 'booting'])
    expect(decodeHandle(handle)).toMatchObject({
      node: n.id,
      nodeName: 'first-0',
      epoch: 1,
      region: 'first',
    })
    expect(await placement(key)).toMatchObject({
      node_id: n.id,
      epoch: '1',
      state: 'placed',
      desired_power: 'running',
      completing: null,
    })
    expect(await running(n.id)).toBe(1024)
    expect(await kinds(key)).toEqual(['placement.created', 'placement.placed'])
    expect(await history(key)).toEqual([{ epoch: '1', node_id: n.id, reason: 'placed', end_reason: null }])
    // Its start answered running, so the wait asks nothing; the same spec again starts with no PUT.
    await runtime.waitRunning(handle, AbortSignal.timeout(1_000))
    const again = await runtime.ensureProvisioned(key, { regionKey: 'first' }, SPEC, quiet)
    expect(decodeHandle(again).epoch).toBe(1)
    expect(await callsSince(since + 2)).toEqual([`first-0 POST /v1/workloads/${key}/start e1 t30000`])
  })

  test('2. a first placement its node refuses for room is undone, and its room given back', async () => {
    const [n] = (await region('refused-put', 1)) as [EnrolledNode]
    const key = server()
    nodes.once(/^PUT /, async () => {
      throw new NodeRefused(507, 'insufficient_capacity', 'not enough memory', null)
    })
    const since = nodes.calls.length
    const made = runtime.ensureProvisioned(key, { regionKey: 'refused-put' }, SPEC, quiet)
    await expect(made).rejects.toBeInstanceOf(RuntimeFull)
    await expect(made).rejects.toThrow('The fleet runtime has no room: refused-put-0: not enough memory')
    expect(await callsSince(since)).toEqual([`refused-put-0 PUT /v1/workloads/${key} e1 t1200000`])
    expect(await placement(key)).toBeUndefined()
    expect(await running(n.id)).toBe(0)
    expect(await history(key)).toEqual([
      {
        epoch: '1',
        node_id: n.id,
        reason: 'placed',
        end_reason: 'refused: insufficient_capacity',
      },
    ])
    expect(await kinds(key)).toEqual(['placement.created', 'placement.abandoned'])
    expect((await eventData('placement.abandoned', key)).map((e) => e.data)).toEqual([
      { code: 'insufficient_capacity' },
    ])
  })

  test('3. a start its node refuses for room gives the claim back and moves the server to start', async () => {
    const [a, b] = (await region('refused-start', 2)) as [EnrolledNode, EnrolledNode]
    const key = server()
    const first = await runtime.ensureProvisioned(key, { regionKey: 'refused-start' }, SPEC, quiet)
    const home = decodeHandle(first).node === a.id ? a : b
    const away = home === a ? b : a
    const homeName = await nameOf(home.id)
    const awayName = await nameOf(away.id)
    await runtime.stop(first)
    nodes.once(/^POST .*\/start$/, async () => {
      throw new NodeRefused(507, 'insufficient_capacity', 'a copy runs here', null)
    })
    const since = nodes.calls.length
    const after = await lastEvent()
    const { sink, seen } = recorder()
    const moved = await runtime.ensureProvisioned(key, { regionKey: 'refused-start' }, SPEC, sink)
    expect(await callsSince(since)).toEqual([
      `${homeName} POST /v1/workloads/${key}/start e1 t30000`,
      `${homeName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${awayName} PUT /v1/workloads/${key} e2 t1200000`,
      `${awayName} POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `${homeName} POST /v1/workloads/${key}/fence e- t${TEARDOWN_MS}`,
      `${homeName} DELETE /v1/workloads/${key}?data=delete e2 t${TEARDOWN_MS}`,
      `${awayName} POST /v1/workloads/${key}/start e2 t60000`,
    ])
    expect(seen).toEqual(['allocating', 'handle e1', 'booting', 'storage', 'compute', 'handle e2', 'booting'])
    expect(decodeHandle(moved)).toMatchObject({ node: away.id, epoch: 2 })
    expect(await running(home.id)).toBe(0)
    expect(await running(away.id)).toBe(1024)
    expect(await kinds(key, after)).toEqual([
      'move.exported',
      'placement.rehomed',
      'placement.moved_for_room',
      'placement.placed',
      'placement.restored',
      'copy.fenced',
    ])
    expect((await eventData('placement.moved_for_room', key)).map((e) => e.data)).toEqual([
      {
        fromNode: home.id,
        fromEpoch: 1,
        why: `${homeName}: a copy runs here`,
      },
    ])
    expect(await archives(key)).toMatchObject([{ purpose: 'move', status: 'deleted', object_key: null }])
    expect(stored(key)).toEqual([])
  })

  test('4. start() on a node with no room now claims nothing and asks no node', async () => {
    const [n] = (await region('full-start', 1)) as [EnrolledNode]
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'full-start' }, SPEC, quiet)
    await runtime.stop(handle)
    await runtime.ensureProvisioned(
      server(),
      { regionKey: 'full-start' },
      { ...SPEC, resources: { memoryMb: 3584 } },
      quiet,
    )
    const since = nodes.calls.length
    const started = runtime.start(handle)
    await expect(started).rejects.toBeInstanceOf(RuntimeFull)
    await expect(started).rejects.toThrow(/^The fleet runtime has no room: full-start-0: /)
    expect(await callsSince(since)).toEqual([])
    expect((await placement(key))?.desired_power).toBe('stopped')
    expect(await running(n.id)).toBe(3584)
  })

  test('5. an upload is tried five times, and one nobody finished is taken over after three hours', async () => {
    const [n] = (await region('attempts', 1)) as [EnrolledNode]
    const key = server()
    const fail = () =>
      nodes.once(/\/upload$/, async () => {
        throw new Error('the store said no')
      })
    // A fourth try that fails leaves it to be sent again.
    const fourth = await snapshotOn(n.id, key, null, { attempts: 3 })
    fail()
    expect(await runtime.upload(fourth.id)).toBe('failed')
    // The fifth is final.
    const fifth = await snapshotOn(n.id, key, null, { attempts: 4 })
    fail()
    expect(await runtime.upload(fifth.id)).toBe('failed')
    const byId = new Map((await archives(key)).map((a) => [a.id, a]))
    expect(byId.get(fourth.id)).toMatchObject({
      status: 'local',
      attempts: 4,
      error: 'the store said no',
    })
    expect(byId.get(fifth.id)).toMatchObject({
      status: 'failed',
      attempts: 5,
      error: 'the store said no',
    })
    expect((await eventData('snapshot.upload_failed', key)).map((e) => e.data)).toEqual([
      { id: fifth.id, error: 'the store said no' },
    ])
    // An upload under way is left to finish; one not heard from in three hours is taken over.
    const fresh = await snapshotOn(n.id, key, null)
    await db.execute(sql`UPDATE fleet_archives SET status = 'uploading', upload_started_at = now() - interval '179 minutes'
                         WHERE id = ${fresh.id}`)
    const stale = await snapshotOn(n.id, key, null)
    await db.execute(sql`UPDATE fleet_archives SET status = 'uploading', upload_started_at = now() - interval '181 minutes'
                         WHERE id = ${stale.id}`)
    const since = nodes.calls.length
    expect(await runtime.upload(fresh.id)).toBe('skipped')
    expect(await runtime.upload(stale.id)).toBe('ready')
    expect(await callsSince(since)).toEqual([
      `attempts-0 POST /v1/workloads/${key}/snapshots/${stale.id}/upload e- t${TRANSFER_MS}`,
    ])
    expect((await archives(key)).find((a) => a.id === stale.id)).toMatchObject({
      status: 'ready',
      attempts: 1,
      object_key: `fleet/test/${key}/${stale.id}.tar.gz`,
    })
  })

  test("6. a node's copy is trimmed only when a newer one of its server reached the store", async () => {
    const [n] = (await region('trim', 1)) as [EnrolledNode]
    const key = server()
    const oldest = await snapshotOn(n.id, key, `fleet/test/${key}/oldest`, {
      capturedAgo: '3 hours',
    })
    const newer = await snapshotOn(n.id, key, `fleet/test/${key}/newer`, {
      capturedAgo: '2 hours',
    })
    const newest = await snapshotOn(n.id, key, null, {
      capturedAgo: '1 hour',
    })
    const alone = await snapshotOn(n.id, server(), `fleet/test/alone`, {
      capturedAgo: '4 hours',
    })
    const since = nodes.calls.length
    expect(await runtime.trimLocalCopies()).toBe(1)
    expect(await callsSince(since)).toEqual([
      `trim-0 DELETE /v1/workloads/${key}/snapshots/${oldest.id} e- t${TEARDOWN_MS}`,
    ])
    const byId = new Map((await archives(key)).map((a) => [a.id, a]))
    expect(byId.get(oldest.id)?.local_deleted_at).not.toBeNull()
    expect(byId.get(oldest.id)).toMatchObject({
      status: 'ready',
      object_key: `fleet/test/${key}/oldest`,
    })
    expect(byId.get(newer.id)?.local_deleted_at).toBeNull()
    expect(byId.get(newest.id)?.local_deleted_at).toBeNull()
    expect(store.objects.has(`fleet/test/${key}/oldest`)).toBe(true)
    expect(alone.id).toBeTruthy()
    expect((await eventData('snapshot.local_trimmed', key)).map((e) => e.data)).toEqual([{ id: oldest.id }])
  })

  test('7. a snapshot, its upload, its deletion, and what upkeep deletes once the store answers', async () => {
    await region('backup', 1)
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'backup' }, SPEC, quiet)
    const since = nodes.calls.length
    const taken = await runtime.snapshot(handle)
    // Its upload runs on its own once the snapshot is taken.
    for (let i = 0; i < 100 && (await archives(key))[0]?.status !== 'ready'; i++)
      await new Promise((resolve) => setTimeout(resolve, 20))
    const [row] = await archives(key)
    if (row === undefined) throw new Error('no archive')
    expect(taken.sizeBytes).toBe(5)
    expect(await callsSince(since)).toEqual([
      `backup-0 GET /v1/workloads/${key} e- t-`,
      `backup-0 POST /v1/workloads/${key}/snapshots e1 t${TRANSFER_MS}`,
      `backup-0 POST /v1/workloads/${key}/snapshots/local-${row.id}/upload e- t${TRANSFER_MS}`,
    ])
    expect(nodes.calls[since + 1]?.body).toEqual({
      id: row.id,
      quiesced: true,
    })
    expect(row).toMatchObject({
      purpose: 'snapshot',
      status: 'ready',
      local_id: `local-${row.id}`,
      object_key: `fleet/test/${key}/${row.id}.tar.gz`,
      attempts: 1,
    })
    expect(await kinds(key)).toEqual([
      'placement.created',
      'placement.placed',
      'snapshot.local',
      'snapshot.uploaded',
    ])
    expect(stored(key)).toEqual([`fleet/test/${key}/${row.id}.tar.gz`])
    expect(await runtime.goneSnapshots([taken.snapshot])).toEqual(new Set())

    // Deleted while the store is down: the node's copy goes now, the store's later.
    store.failing = true
    const deleting = nodes.calls.length
    await runtime.deleteSnapshot(taken.snapshot)
    expect(await callsSince(deleting)).toEqual([
      `backup-0 DELETE /v1/workloads/${key}/snapshots/local-${row.id} e- t${TEARDOWN_MS}`,
    ])
    expect((await archives(key))[0]).toMatchObject({
      status: 'deleted',
      object_key: `fleet/test/${key}/${row.id}.tar.gz`,
    })
    expect((await archives(key))[0]?.local_deleted_at).not.toBeNull()
    expect(logs.map((l) => l.message)).toContain('snapshot copies left for upkeep')
    expect(await runtime.goneSnapshots([taken.snapshot])).toEqual(new Set([taken.snapshot]))
    await runtime.dropDeletedCopies()
    expect(stored(key)).toHaveLength(1)
    store.failing = false
    await runtime.dropDeletedCopies()
    expect(stored(key)).toEqual([])
    expect((await archives(key))[0]).toMatchObject({
      status: 'deleted',
      object_key: null,
    })
    expect(await callsSince(deleting)).toHaveLength(1)
  })

  test('8. a planned move: stop, export, a new home restored, then the old copy fenced and deleted', async () => {
    const { key, handle, from, to } = await moving('planned')
    const fromName = await nameOf(from.id)
    const toName = await nameOf(to.id)
    const since = nodes.calls.length
    const after = await lastEvent()
    const { sink, seen } = recorder()
    const moved = await runtime.relocate(handle, { regionKey: 'planned' }, SPEC, sink)
    expect(await callsSince(since)).toEqual([
      `${fromName} GET /v1/workloads/${key} e- t-`,
      `${fromName} POST /v1/workloads/${key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${toName} PUT /v1/workloads/${key} e2 t1200000`,
      `${toName} POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${key}/fence e- t${TEARDOWN_MS}`,
      `${fromName} DELETE /v1/workloads/${key}?data=delete e2 t${TEARDOWN_MS}`,
    ])
    const calls = nodes.calls.slice(since)
    const [archive] = await archives(key)
    if (archive === undefined) throw new Error('no archive')
    const objectKey = `fleet/test/${key}/${archive.id}.tar.gz`
    expect(calls[2]?.body).toEqual({
      quiesced: false,
      exclude: ['libraries'],
      url: expect.stringContaining(encodeURIComponent(objectKey)),
      headers: {},
    })
    expect(calls[4]?.body).toEqual({
      url: expect.stringContaining(encodeURIComponent(objectKey)),
      sha256: 'a'.repeat(64),
    })
    expect(calls[5]?.body).toEqual({ currentEpoch: 2 })
    expect(calls[6]?.headers).toEqual({ [CONFIRM]: key })
    expect(seen).toEqual(['storage', 'compute', 'handle e2'])
    expect(decodeHandle(moved)).toMatchObject({
      node: to.id,
      nodeName: toName,
      epoch: 2,
    })
    expect(await placement(key)).toMatchObject({
      node_id: to.id,
      epoch: '2',
      state: 'placed',
      desired_power: 'running',
      completing: null,
      restore_from: null,
      move_requested_at: null,
    })
    expect(await running(from.id)).toBe(0)
    expect(await running(to.id)).toBe(1024)
    expect(await history(key)).toEqual([
      { epoch: '1', node_id: from.id, reason: 'placed', end_reason: 'moved' },
      { epoch: '2', node_id: to.id, reason: 'move', end_reason: null },
    ])
    expect(await kinds(key, after)).toEqual([
      'move.exported',
      'placement.rehomed',
      'placement.placed',
      'placement.restored',
      'copy.fenced',
    ])
    expect(archive).toMatchObject({
      purpose: 'move',
      status: 'deleted',
      object_key: null,
      attempts: 1,
    })
    expect(stored(key)).toEqual([])
    expect(nodes.copies.has(`${from.id}/${key}`)).toBe(false)
    expect(nodes.copies.get(`${to.id}/${key}`)).toMatchObject({
      epoch: 2,
      state: 'created',
    })
  })

  test('9. a move with no room once its data is copied: the copy goes, and it runs where it was', async () => {
    const { key, handle, from, to } = await moving('no-room-after')
    const fromName = await nameOf(from.id)
    // The node it was to move to stops taking sandboxes while the data is copied.
    nodes.once(/^POST .*\/export$/, async () => {
      await drain(db, to.id, 'test')
    })
    const since = nodes.calls.length
    const after = await lastEvent()
    const moved = runtime.relocate(handle, { regionKey: 'no-room-after' }, SPEC, quiet)
    await expect(moved).rejects.toBeInstanceOf(RuntimeFull)
    // Every node is considered, in name order: the one it was asked to go to is draining now.
    const toName = await nameOf(to.id)
    const others = await rows<{ name: string }>(db, sql`SELECT name FROM fleet_nodes ORDER BY name`)
    const reasons = others.map(({ name }) =>
      name === fromName
        ? `${name}: excluded`
        : name === toName
          ? `${name}: lifecycle draining`
          : `${name}: not the node asked for`,
    )
    await expect(moved).rejects.toThrow(`The fleet runtime has no room: ${reasons.join('; ')}`)
    expect(await callsSince(since)).toEqual([
      `${fromName} GET /v1/workloads/${key} e- t-`,
      `${fromName} POST /v1/workloads/${key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${key}/start e1 t60000`,
    ])
    expect(await placement(key)).toMatchObject({
      node_id: from.id,
      epoch: '1',
      state: 'placed',
      desired_power: 'running',
      completing: null,
    })
    expect((await placement(key))?.move_requested_at).not.toBeNull()
    expect(await running(from.id)).toBe(1024)
    expect(await running(to.id)).toBe(0)
    expect(await history(key)).toEqual([{ epoch: '1', node_id: from.id, reason: 'placed', end_reason: null }])
    expect(await kinds(key, after)).toEqual(['move.exported'])
    expect(await archives(key)).toMatchObject([{ purpose: 'move', status: 'deleted', object_key: null }])
    expect(stored(key)).toEqual([])
    expect(nodes.copies.get(`${from.id}/${key}`)).toMatchObject({
      epoch: 1,
      state: 'running',
    })
  })

  test('10. a move whose old copy its node refuses to fence is finished, and the copy left for upkeep', async () => {
    const refused = await moving('fence-refused')
    nodes.once(/^POST .*\/fence$/, async () => {
      throw new NodeRefused(409, 'busy', 'not now', null)
    })
    const fromName = await nameOf(refused.from.id)
    const since = nodes.calls.length
    const moved = await runtime.relocate(refused.handle, { regionKey: 'fence-refused' }, SPEC, quiet)
    expect(decodeHandle(moved)).toMatchObject({
      node: refused.to.id,
      epoch: 2,
    })
    expect((await callsSince(since)).filter((c) => c.startsWith(fromName))).toEqual([
      `${fromName} GET /v1/workloads/${refused.key} e- t-`,
      `${fromName} POST /v1/workloads/${refused.key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${refused.key}/export e1 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${refused.key}/fence e- t${TEARDOWN_MS}`,
    ])
    expect(logs.at(-1)).toEqual({
      message: 'move left its source copy for upkeep',
      fields: {
        node: refused.from.id,
        workload: refused.key,
        error: 'not now',
      },
    })
    expect(await eventData('copy.fenced', refused.key)).toEqual([])

    // Unreachable, the same; anything else is not the node's answer, and fails the move.
    const unreachable = await moving('fence-unreachable')
    nodes.once(/^POST .*\/fence$/, async () => {
      throw new NodeUnreachable('timeout', 'no answer')
    })
    await runtime.relocate(unreachable.handle, { regionKey: 'fence-unreachable' }, SPEC, quiet)
    expect(logs.at(-1)?.message).toBe('move left its source copy for upkeep')
    const broken = await moving('fence-broken')
    nodes.once(/^POST .*\/fence$/, async () => {
      throw new Error('a bug')
    })
    await expect(runtime.relocate(broken.handle, { regionKey: 'fence-broken' }, SPEC, quiet)).rejects.toThrow(
      'a bug',
    )
    expect(await placement(broken.key)).toMatchObject({
      node_id: broken.to.id,
      epoch: '2',
      state: 'placed',
    })
  })

  test('11. a restore in place: a new epoch, the copy stopped, filled from its local snapshot, started', async () => {
    const [n] = (await region('restore', 1)) as [EnrolledNode]
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'restore' }, SPEC, quiet)
    const snap = await snapshotOn(n.id, key, null)
    const since = nodes.calls.length
    const after = await lastEvent()
    const { sink, seen } = recorder()
    const restored = await runtime.restore(handle, { kind: 'snapshot', snapshot: snap.handle }, SPEC, sink)
    expect(await callsSince(since)).toEqual([
      `restore-0 GET /v1/workloads/${key} e- t-`,
      `restore-0 POST /v1/workloads/${key}/stop e2 t${TEARDOWN_MS}`,
      `restore-0 PUT /v1/workloads/${key} e2 t1200000`,
      `restore-0 POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `restore-0 POST /v1/workloads/${key}/start e2 t60000`,
    ])
    expect(nodes.calls[since + 3]?.body).toEqual({ snapshot: snap.id })
    expect(seen).toEqual(['storage', 'compute', 'handle e2'])
    expect(decodeHandle(restored)).toMatchObject({ node: n.id, epoch: 2 })
    expect(await placement(key)).toMatchObject({
      epoch: '2',
      state: 'placed',
      desired_power: 'running',
    })
    expect(await history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'restored' },
      { epoch: '2', node_id: n.id, reason: 'restore', end_reason: null },
    ])
    expect(await kinds(key, after)).toEqual(['placement.rehomed', 'placement.placed', 'placement.restored'])
    // The old handle names a placement that is gone.
    await expect(runtime.stop(handle)).rejects.toThrow('This handle is for epoch 1; the server is at 2')
  })

  test('12. a release deletes its copy and keeps its backups; a destroy deletes everything', async () => {
    const [n] = (await region('letting-go', 1)) as [EnrolledNode]
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'letting-go' }, SPEC, quiet)
    const objectKey = `fleet/test/${key}/kept`
    const snap = await snapshotOn(n.id, key, objectKey)
    await pulse(n)
    const since = nodes.calls.length
    const after = await lastEvent()
    const released = await runtime.release(handle)
    expect(await callsSince(since)).toEqual([
      `letting-go-0 DELETE /v1/workloads/${key}?data=delete e1 t${TEARDOWN_MS}`,
    ])
    expect(nodes.calls[since]?.headers).toEqual({ [CONFIRM]: key })
    expect(decodeHandle(released)).toMatchObject({
      node: null,
      nodeName: null,
      epoch: 1,
      edgeHost: null,
    })
    expect(await placement(key)).toMatchObject({
      node_id: null,
      epoch: '1',
      state: 'released',
      completing: null,
    })
    expect(await history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'released' },
    ])
    expect((await archives(key))[0]).toMatchObject({
      id: snap.id,
      status: 'ready',
      object_key: objectKey,
    })
    expect((await archives(key))[0]?.local_deleted_at).not.toBeNull()
    expect(stored(key)).toEqual([objectKey])

    // Its node still reports the copy it held until its next beat: destroy asks it too.
    const destroying = nodes.calls.length
    await runtime.destroy(released)
    expect(await callsSince(destroying)).toEqual([
      `letting-go-0 DELETE /v1/workloads/${key}?data=delete e1 t${TEARDOWN_MS}`,
    ])
    expect(await placement(key)).toBeUndefined()
    expect(await history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'released' },
    ])
    expect(await archives(key)).toMatchObject([{ status: 'deleted', object_key: objectKey }])
    expect(stored(key)).toEqual([])
    expect(await kinds(key, after)).toEqual(['placement.released', 'placement.destroyed'])
    expect((await eventData('placement.destroyed', key)).map((e) => e.data)).toEqual([{ nodes: [n.id] }])
  })

  test('13. a suspended copy whose resume does not come back goes on as a boot', async () => {
    const [n] = (await region('resume-hung', 1)) as [EnrolledNode]
    const key = server()
    await runtime.ensureProvisioned(key, { regionKey: 'resume-hung' }, SPEC, quiet)
    Object.assign(nodes.copies.get(`${n.id}/${key}`) ?? {}, {
      state: 'suspended',
    })
    await pulse(n)
    nodes.once(/^POST .*\/start$/, async (answer) => ({
      ...(answer() as object),
      resumed: false,
    }))
    const { sink, seen } = recorder()
    await runtime.ensureProvisioned(key, { regionKey: 'resume-hung' }, SPEC, sink)
    expect(seen).toEqual(['allocating', 'handle e1', 'resuming', 'booting'])
  })
})
