// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Moves between nodes that fail part-way and are tried again: what a move leaves in the archive
 * store, what happens to the snapshots on the node it leaves, and where a data goes when a node
 * refuses it or comes back after it was declared lost. Its pryxd in memory is a smaller copy of
 * fleet-runtime.test.ts's, for the reason fleet-runtime.flows.test.ts gives.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { FleetStore } from '../src/archive-transport.ts'
import type { FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { decodeHandle, encodeSnapshot } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import {
  type CallOptions,
  type NodeAddress,
  type NodeClient,
  NodeRefused,
  NodeUnreachable,
} from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { confirmLost, drain, heartbeat, reinstate, undrain } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import {
  type ProgressSink,
  RuntimeFull,
  type RuntimeHandle,
  type RuntimeKey,
  type RuntimeSpec,
  runtimeKey,
} from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import type { WorkloadReport, WorkloadState, WorkloadView } from '../src/wire.ts'
import { MemoryStore } from './memory-store.ts'
import {
  beat,
  capacity,
  type EnrolledNode,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  report,
  testCa,
} from './nodes.ts'

/** The state each power command leaves a copy in. */
const POWER: Record<string, WorkloadState> = {
  '/start': 'running',
  '/stop': 'stopped',
  '/kill': 'stopped',
  '/fence': 'fenced',
}
const RESTORED = {
  sizeBytes: 5,
  sha256: null,
  entries: 1,
  skipped: 0,
  unpackedBytes: 5,
  previousData: null,
  durationMs: 1,
}

interface Copy {
  epoch: number
  state: WorkloadState
  ports: Record<string, number>
}

/**
 * pryxd as the runtime calls it: the copies each node holds, and every call. `once` has the
 * next call that matches run something first, which may throw instead; `lost` answers the next
 * call that matches, then loses the answer; a node in `full` refuses every copy for room.
 */
class Nodes {
  readonly calls: string[] = []
  readonly copies = new Map<string, Copy>()
  readonly full = new Set<string>()
  readonly #store: MemoryStore
  readonly #hooks: Array<{ match: RegExp; run: () => Promise<void> }> = []
  readonly #lost: RegExp[] = []
  #port = 30_000

  constructor(store: MemoryStore) {
    this.#store = store
  }

  once(match: RegExp, run: () => Promise<void>): void {
    this.#hooks.push({ match, run })
  }

  lost(match: RegExp): void {
    this.#lost.push(match)
  }

  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    const line = `${method} ${path}`
    this.calls.push(`${node.id} ${line}`)
    const at = this.#hooks.findIndex((hook) => hook.match.test(line))
    if (at >= 0) await this.#hooks.splice(at, 1)[0]?.run()
    const answer = this.#answer(node.id, method, path, options) as T
    const lost = this.#lost.findIndex((match) => match.test(line))
    if (lost >= 0) {
      this.#lost.splice(lost, 1)
      throw new NodeUnreachable('reset', 'connection reset')
    }
    return answer
  }

  async probe() {
    return { tcp: 'open' as const, api: 'ok', ms: 1 }
  }

  close(): void {}

  reports(node: string): WorkloadReport[] {
    return [...this.copies]
      .filter(([where]) => where.startsWith(`${node}/`))
      .map(([where, copy]) =>
        report(where.slice(node.length + 1), copy.epoch, {
          state: copy.state,
          ports: copy.ports,
        }),
      )
  }

  #answer(node: string, method: string, path: string, options: CallOptions): unknown {
    const [, workload = '', rest = ''] = /^\/v1\/workloads\/([^/?]+)(.*)$/.exec(path) ?? []
    const where = `${node}/${workload}`
    const copy = this.copies.get(where)
    const power = POWER[rest]
    if (method === 'GET' && rest === '') return this.#view(workload, copy)
    if (method === 'PUT') return this.#put(node, where, workload, copy, options.epoch ?? 0)
    if (method === 'POST' && power !== undefined) {
      if (copy !== undefined) copy.state = power
      return {
        changed: true,
        forced: false,
        stopped: false,
        workload: this.#view(workload, copy),
      }
    }
    if (method === 'POST' && (rest === '/export' || /^\/snapshots\/[^/]+\/upload$/.test(rest)))
      return this.#send(options)
    if (method === 'POST' && rest === '/restore') return RESTORED
    if (method === 'DELETE') return this.#delete(where, rest, copy)
    throw new Error(`this node doesn't answer ${method} ${path}`)
  }

  /** An archive sent where the call says: the URL is the store's own for one object. */
  #send(options: CallOptions) {
    const url = (options.body as { url?: string } | undefined)?.url ?? ''
    this.#store.objects.set(decodeURIComponent(url.slice(url.lastIndexOf('/') + 1)), Buffer.from('data!'))
    return {
      sizeBytes: 5,
      sha256: 'a'.repeat(64),
      format: 'tar.gz',
      entries: 1,
      durationMs: 1,
    }
  }

  /** A snapshot's copy dropped, or a copy deleted with its data, into the node's trash. */
  #delete(where: string, rest: string, copy: Copy | undefined) {
    if (rest.startsWith('/snapshots/')) return { existed: true }
    this.copies.delete(where)
    return {
      existed: copy !== undefined,
      removedContainer: true,
      data: 'trashed',
      trashPath: null,
    }
  }

  #put(node: string, where: string, workload: string, copy: Copy | undefined, epoch: number) {
    if (this.full.has(node))
      throw new NodeRefused(409, 'insufficient_capacity', 'not enough memory', undefined)
    const made = {
      epoch,
      state: copy?.state ?? 'created',
      ports: copy?.ports ?? { game: this.#port++ },
    }
    this.copies.set(where, made)
    return {
      outcome: copy ? 'replaced' : 'created',
      restarted: false,
      workload: this.#view(workload, made),
    }
  }

  #view(workload: string, copy: Copy | undefined): WorkloadView {
    if (copy === undefined) throw new NodeRefused(404, 'not_found', 'no such workload', null)
    return {
      id: workload,
      generation: 1,
      epoch: copy.epoch,
      supersededBy: null,
      specDigest: 'd',
      image: 'alpine:3.22',
      state: copy.state,
      exit: null,
      restartCount: 0,
      lastFailureAt: null,
      startedAt: null,
      finishedAt: null,
      changedAt: new Date().toISOString(),
      ports: Object.entries(copy.ports).map(([name, hostPort]) => ({
        name,
        hostPort,
        containerPort: 8080,
        endpoints: { edge: [], control: [] },
      })),
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
  }
}

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
const progress: ProgressSink = { step: async () => {}, handle: async () => {} }

let db: Db
let drop: () => Promise<void>
let ca: FleetCa
let store: MemoryStore
let nodes: Nodes
let runtime: FleetRuntime
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
const events = (kind: string, workload: string) =>
  rows<{ node_id: string | null; data: Record<string, unknown> }>(
    db,
    sql`SELECT node_id, data FROM fleet_events WHERE kind = ${kind} AND workload = ${workload}`,
  )
const archive = async (id: string) =>
  (
    await rows<{
      status: string
      object_key: string | null
      local_deleted_at: Date | null
    }>(db, sql`SELECT status, object_key, local_deleted_at FROM fleet_archives WHERE id = ${id}`)
  )[0]
const moveExports = (workload: string) =>
  rows<{ status: string; object_key: string | null }>(
    db,
    sql`SELECT status, object_key FROM fleet_archives WHERE workload = ${workload} AND purpose = 'move'`,
  )
const stored = (workload: string) => [...store.objects.keys()].filter((key) => key.includes(workload))

/** A snapshot only on `node`, not uploaded yet, or in the store too when `objectKey` is given. */
async function snapshotOn(node: string, workload: string, objectKey: string | null = null) {
  const id = randomUUID()
  await db.execute(sql`
    INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, local_id, object_key, status, consistency, captured_at)
    VALUES (${id}, ${workload}, 'snapshot', 1, ${node}, ${id}, ${objectKey},
            ${objectKey === null ? 'local' : 'ready'}, 'stopped', now())`)
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

/** A server placed and running on one of the region's two nodes, with a move to the other asked for. */
async function moving(name: string) {
  const [a, b] = (await region(name, 2)) as [EnrolledNode, EnrolledNode]
  const key = server()
  const handle: RuntimeHandle = await runtime.ensureProvisioned(key, { regionKey: name }, SPEC, progress)
  const [from, to] = decodeHandle(handle).node === a.id ? [a, b] : [b, a]
  await runtime.requestMove(key, to.id, 'operator:test')
  return { key, handle, from, to }
}

/** A database, a store and nodes of their own, for one block of tests. */
async function start(): Promise<void> {
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
  store = new MemoryStore()
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
    log: () => {},
  })
}

async function stop(): Promise<void> {
  store.close()
  await drop?.()
}

describe.skipIf(!hasDatabase)('FleetRuntime, the copies a failed or finished move leaves', () => {
  beforeAll(start)
  afterAll(stop)

  test('a snapshot not yet uploaded is uploaded before a move deletes the copy it leaves', async () => {
    const { key, handle, from, to } = await moving('pre-move')
    const snap = await snapshotOn(from.id, key)
    const moved = await runtime.relocate(handle, { regionKey: 'pre-move' }, SPEC, progress)
    expect(decodeHandle(moved).node).toBe(to.id)
    expect(nodes.copies.has(`${from.id}/${key}`)).toBe(false)
    const row = await archive(snap.id)
    expect(row?.status).toBe('ready')
    expect(store.objects.has(row?.object_key ?? '')).toBe(true)
    expect(await runtime.goneSnapshots([snap.handle])).toEqual(new Set())
  })

  test('a snapshot that can’t be uploaded is no longer offered once the copy it was on is deleted', async () => {
    const { key, handle, from } = await moving('pre-move-lost')
    const snap = await snapshotOn(from.id, key)
    // Too big for any upload, and not saying how big: it never reaches the store.
    nodes.once(/\/upload$/, async () => {
      throw new NodeRefused(422, 'archive_too_large', 'too large', undefined)
    })
    await runtime.relocate(handle, { regionKey: 'pre-move-lost' }, SPEC, progress)
    expect(nodes.copies.has(`${from.id}/${key}`)).toBe(false)
    expect((await archive(snap.id))?.local_deleted_at).not.toBeNull()
    expect(await runtime.goneSnapshots([snap.handle])).toEqual(new Set([snap.handle]))
    expect((await events('snapshot.lost_with_copy', key)).map((e) => e.data.id)).toEqual([snap.id])
  })

  test('a move’s copy whose export failed after reaching the store is removed', async () => {
    const { key, handle } = await moving('lost-export')
    nodes.lost(/^POST .*\/export$/)
    await expect(
      runtime.relocate(handle, { regionKey: 'lost-export' }, SPEC, progress),
    ).rejects.toBeInstanceOf(NodeUnreachable)
    for (let pass = 0; pass < 3; pass++) await runtime.upkeep()
    expect(stored(key)).toEqual([])
    expect(await moveExports(key)).toEqual([{ status: 'deleted', object_key: null }])
  })

  test('a move’s copy whose export never finished is removed by upkeep once it is stale', async () => {
    const [n] = (await region('stuck-export', 1)) as [EnrolledNode]
    const key = server()
    const objectKey = `fleet/test/${key}/move.tar.gz`
    store.objects.set(objectKey, Buffer.from('data!'))
    await db.execute(sql`
      INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, object_key, status, consistency, captured_at,
        attempts, upload_started_at)
      VALUES (${randomUUID()}, ${key}, 'move', 1, ${n.id}, ${objectKey}, 'uploading', 'stopped', now(), 1,
              now() - interval '4 hours')`)
    await runtime.upkeep()
    expect(stored(key)).toEqual([])
    expect(await moveExports(key)).toEqual([{ status: 'deleted', object_key: null }])
  })
})

describe.skipIf(!hasDatabase)('FleetRuntime, a new home that fails part-way', () => {
  beforeAll(start)
  afterAll(stop)

  test('a move target declared lost and reinstated finishes the move at the next start', async () => {
    const { key, handle, from, to } = await moving('reinstated')
    // The new home is chosen and made, and the data never reaches it.
    nodes.once(/^POST .*\/restore$/, async () => {
      throw new NodeUnreachable('reset', 'connection reset')
    })
    await expect(
      runtime.relocate(handle, { regionKey: 'reinstated' }, SPEC, progress),
    ).rejects.toBeInstanceOf(NodeUnreachable)
    await pulse(from)
    await db.execute(
      sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - interval '10 minutes' WHERE id = ${to.id}`,
    )
    await confirmLost(db, THRESHOLDS, to.id, {
      fencedBy: 'test',
      reason: 'test',
      by: 'test',
    })
    await reinstate(db, to.id, 'test')
    await pulse(to)
    await runtime.refreshRegistry()
    // The copy the move left is the data until the move finishes.
    await runtime.upkeep()
    expect(nodes.copies.has(`${from.id}/${key}`)).toBe(true)
    const started = await runtime.ensureProvisioned(key, { regionKey: 'reinstated' }, SPEC, progress)
    expect(decodeHandle(started).node).toBe(to.id)
    expect(nodes.copies.get(`${to.id}/${key}`)?.state).toBe('running')
    expect(nodes.copies.has(`${from.id}/${key}`)).toBe(false)
  })

  test('a wake its node refuses for room goes to another node once one has room', async () => {
    const [a, b] = (await region('refused-wake', 2)) as [EnrolledNode, EnrolledNode]
    const key = server()
    const placed = await runtime.ensureProvisioned(key, { regionKey: 'refused-wake' }, SPEC, progress)
    const snap = await snapshotOn(decodeHandle(placed).node ?? '', key, `fleet/test/${key}/rest.tar.gz`)
    await runtime.stop(placed)
    const resting = await runtime.release(placed)
    // Only a has room in the ledger, and a refuses it.
    await drain(db, b.id, 'test')
    nodes.full.add(a.id)
    const from = { kind: 'snapshot' as const, snapshot: snap.handle }
    await expect(runtime.restore(resting, from, SPEC, progress)).rejects.toBeInstanceOf(RuntimeFull)
    await undrain(db, b.id, 'test')
    const woken = await runtime.restore(resting, from, SPEC, progress)
    expect(decodeHandle(woken).node).toBe(b.id)
    expect((await events('placement.refused', key)).map((e) => e.node_id)).toEqual([a.id, a.id])
    nodes.full.delete(a.id)
  })
})
