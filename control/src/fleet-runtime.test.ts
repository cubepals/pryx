// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  type ProgressSink,
  RuntimeFull,
  type RuntimeHandle,
  type RuntimeKey,
  type RuntimeSpec,
  RuntimeUnsupported,
  runtimeKey,
} from '../../app/ports/runtime.ts'
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
} from '../../testing/fleet.ts'
import { MemoryStore } from '../../testing/memory-store.ts'
import type { FleetCa } from './ca.ts'
import { FleetRuntime, StaleHandle } from './fleet-runtime.ts'
import { decodeHandle, encodeSnapshot } from './handle.ts'
import { THRESHOLDS } from './health.ts'
import {
  type CallOptions,
  type NodeAddress,
  type NodeClient,
  NodeRefused,
  NodeUnreachable,
} from './node-client.ts'
import { DEFAULTS } from './placement.ts'
import { confirmLost, drain, heartbeat, nodeSummaries, undrain } from './registry.ts'
import { rows } from './sql.ts'
import { FleetStore } from './store.ts'
import type { WorkloadReport, WorkloadState, WorkloadView } from './wire.ts'

/**
 * FleetRuntime against Postgres, with blocklyd in memory: the paths a real node can't be made to
 * take on cue, such as a request that never reaches it, a copy too big to upload, or a placement
 * that changes while a world is copied. fleet-runtime.e2e.test.ts runs it against a real blocklyd.
 */

/** A store whose deletes fail while `failing` is set, as one that is down does. */
class FlakyStore extends MemoryStore {
  failing = false

  override async delete(key: string): Promise<void> {
    if (this.failing) throw new Error('the store is down')
    await super.delete(key)
  }
}

interface Copy {
  epoch: number
  state: WorkloadState
  ports: Record<string, number>
}

/**
 * blocklyd as the runtime calls it: the copies each node holds, what reaches the store, and every
 * call. `once` has the next call that matches run something first, which may throw instead.
 */
class Nodes {
  readonly calls: string[] = []
  readonly copies = new Map<string, Copy>()
  /** What any archive a node packs holds, and the most it sends in one PUT; larger goes in parts. */
  archive = Buffer.from('world')
  maxPut = Number.POSITIVE_INFINITY
  readonly #store: MemoryStore
  readonly #hooks: Array<{ match: RegExp; run: () => Promise<void> }> = []
  #port = 30_000

  constructor(store: MemoryStore) {
    this.#store = store
  }

  once(match: RegExp, run: () => Promise<void>): void {
    this.#hooks.push({ match, run })
  }

  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    const line = `${method} ${path}`
    this.calls.push(`${node.id} ${line}`)
    const at = this.#hooks.findIndex((hook) => hook.match.test(line))
    if (at >= 0) await this.#hooks.splice(at, 1)[0]?.run()
    return this.#answer(node.id, method, path, options) as T
  }

  async probe() {
    return { tcp: 'open' as const, api: 'ok', ms: 1 }
  }

  close(): void {}

  /** What a node's heartbeat says of the copies it holds. */
  reports(node: string): WorkloadReport[] {
    return [...this.copies]
      .filter(([where]) => where.startsWith(`${node}/`))
      .map(([where, copy]) =>
        report(where.slice(node.length + 1), copy.epoch, { state: copy.state, ports: copy.ports }),
      )
  }

  #answer(node: string, method: string, path: string, options: CallOptions): unknown {
    const [, workload = '', rest = ''] = /^\/v1\/workloads\/([^/?]+)(.*)$/.exec(path) ?? []
    const where = `${node}/${workload}`
    const copy = this.copies.get(where)
    const body = (options.body ?? {}) as {
      url?: string
      id?: string
      parts?: { partSize: number; urls: string[] }
    }
    const power = (state: WorkloadState) => {
      if (copy !== undefined) copy.state = state
      return { changed: true, forced: false, workload: this.#view(workload, copy) }
    }
    if (method === 'GET' && rest === '') return this.#view(workload, copy)
    if (method === 'PUT') {
      const made = {
        epoch: options.epoch ?? 0,
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
    if (method === 'POST' && rest === '/start') return power('running')
    if (method === 'POST' && (rest === '/stop' || rest === '/kill')) return power('stopped')
    if (method === 'POST' && rest === '/fence') return { ...power('fenced'), stopped: false }
    if (method === 'POST' && (rest === '/export' || /^\/snapshots\/[^/]+\/upload$/.test(rest))) {
      const bytes = this.archive
      const sent = {
        sizeBytes: bytes.length,
        sha256: 'a'.repeat(64),
        format: 'tar.gz',
        entries: 1,
        durationMs: 1,
      }
      const refuse = (limitBytes: number) =>
        new NodeRefused(422, 'archive_too_large', 'too large', { sizeBytes: bytes.length, limitBytes })
      if (bytes.length <= this.maxPut) {
        // The URL is the store's own for one object: what lands there is what the node sent.
        const url = body.url ?? ''
        this.#store.objects.set(decodeURIComponent(url.slice(url.lastIndexOf('/') + 1)), bytes)
        return sent
      }
      // As blocklyd does: in parts when they are offered and are enough, else refused unsent.
      const parts = body.parts
      if (parts === undefined) throw refuse(this.maxPut)
      const needed = Math.max(1, Math.ceil(bytes.length / parts.partSize))
      if (needed > parts.urls.length) throw refuse(parts.partSize * parts.urls.length)
      const put = parts.urls.slice(0, needed).map((url, index) => {
        const [, upload = '', number = ''] = /__parts\/([^/]+)\/(\d+)$/.exec(url) ?? []
        const chunk = bytes.subarray(index * parts.partSize, (index + 1) * parts.partSize)
        return { number: index + 1, etag: this.#store.putPart(upload, Number(number), chunk) }
      })
      return { ...sent, parts: put }
    }
    if (method === 'POST' && rest === '/restore')
      return {
        sizeBytes: 5,
        sha256: null,
        entries: 1,
        skipped: 0,
        unpackedBytes: 5,
        previousData: null,
        durationMs: 1,
      }
    if (method === 'DELETE' && rest.startsWith('/snapshots/')) return { existed: true }
    if (method === 'DELETE') {
      this.copies.delete(where)
      return { existed: copy !== undefined, removedContainer: true, data: 'trashed', trashPath: null }
    }
    throw new Error(`this node doesn't answer ${method} ${path}`)
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
        containerPort: 25565,
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
  ports: [{ name: 'game', port: 25565, protocol: 'tcp', audience: ['edge', 'control'] }],
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
/** A node of an earlier blocklyd, which sends an archive in one PUT only. */
const ONE_PUT_ONLY = FEATURES.filter((feature) => feature !== 'multipart-upload')
const progress: ProgressSink = { step: async () => {}, handle: async () => {} }
const TOO_LARGE = () =>
  new NodeRefused(422, 'archive_too_large', 'the archive is too large for one upload', {
    sizeBytes: 6 * 1024 ** 3,
    limitBytes: 5 * 1024 ** 3,
  })

describe.skipIf(!hasDatabase)('FleetRuntime, with blocklyd in memory', () => {
  let db: Db
  let drop: () => Promise<void>
  let ca: FleetCa
  let store: FlakyStore
  let nodes: Nodes
  let runtime: FleetRuntime
  let seq = 0

  /** A heartbeat with what the node holds, as blocklyd sends one every few seconds. */
  const pulse = (n: EnrolledNode, features = FEATURES) =>
    heartbeat(
      db,
      registryOptions(),
      n.id,
      n.sha,
      beat(n, 'session', ++seq, nodes.reports(n.id), {
        features,
        capacity: capacity({ allocatableMemoryMb: 4096 }),
      }),
    )

  /** Nodes of their own region, healthy, so each test places only on its own. */
  async function region(name: string, count: number, features = FEATURES): Promise<EnrolledNode[]> {
    const made: EnrolledNode[] = []
    for (let i = 0; i < count; i++) {
      const n = await enrollNode(db, ca, `${name}-${i}`, { region: name })
      await pulse(n, features)
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
  const archives = (workload: string) =>
    rows<{ status: string; object_key: string | null; error: string | null; local_deleted_at: Date | null }>(
      db,
      sql`SELECT status, object_key, error, local_deleted_at FROM fleet_archives WHERE workload = ${workload}`,
    )
  const ledger = async (node: string) =>
    (await nodeSummaries(db, THRESHOLDS, { includeRetired: true })).find((n) => n.id === node)
  const stored = (workload: string) => [...store.objects.keys()].filter((key) => key.includes(workload))

  /** A snapshot the node holds a copy of, also uploaded to the store when `objectKey` is given. */
  async function snapshotOn(node: string, workload: string, objectKey: string | null) {
    const id = randomUUID()
    await db.execute(sql`
      INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, local_id, object_key, status, consistency, captured_at)
      VALUES (${id}, ${workload}, 'snapshot', 1, ${node}, ${id}, ${objectKey},
              ${objectKey === null ? 'local' : 'ready'}, 'stopped', now())`)
    if (objectKey !== null) store.objects.set(objectKey, Buffer.from('world'))
    return { id, handle: encodeSnapshot({ deployment: 'test', key: workload, archive: id, epoch: 1 }) }
  }

  /** A server placed and running on one of the region's nodes, with a move to the other asked for. */
  async function moving(
    name: string,
    features = FEATURES,
  ): Promise<{ key: RuntimeKey; handle: RuntimeHandle; to: string }> {
    const [a, b] = await region(name, 2, features)
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: name }, SPEC, progress)
    const to = decodeHandle(handle).node === a?.id ? (b?.id ?? '') : (a?.id ?? '')
    await runtime.requestMove(key, to, 'operator:test')
    return { key, handle, to }
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
      log: () => {},
    })
  })
  afterAll(async () => {
    store.close()
    await drop?.()
  })

  test('a move whose placement changed while its world was copied discards the copy', async () => {
    const { key, handle } = await moving('stale-move')
    // Released meanwhile, with its epoch unchanged, after the copy was checked against it.
    nodes.once(/^POST .*\/export$/, async () => {
      await db.execute(
        sql`UPDATE fleet_placements SET state = 'released', node_id = NULL WHERE workload = ${key}`,
      )
    })
    await expect(
      runtime.relocate(handle, { regionKey: 'stale-move' }, SPEC, progress),
    ).rejects.toBeInstanceOf(StaleHandle)
    expect(await archives(key)).toMatchObject([{ status: 'deleted', object_key: null }])
    expect(stored(key)).toEqual([])
  })

  test('so does a move to make room to start', async () => {
    const [a, b] = await region('stale-room', 2)
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'stale-room' }, SPEC, progress)
    const home = decodeHandle(handle).node === a?.id ? (a as EnrolledNode) : (b as EnrolledNode)
    const away = home === a ? (b as EnrolledNode) : (a as EnrolledNode)
    await runtime.stop(handle)
    // Another server takes its node's room while it sleeps.
    await drain(db, away.id, 'test')
    await runtime.ensureProvisioned(
      server(),
      { regionKey: 'stale-room' },
      { ...SPEC, resources: { memoryMb: 3584 } },
      progress,
    )
    await undrain(db, away.id, 'test')
    nodes.once(/^POST .*\/export$/, async () => {
      await db.execute(
        sql`UPDATE fleet_placements SET state = 'released', node_id = NULL WHERE workload = ${key}`,
      )
    })
    await expect(
      runtime.ensureProvisioned(key, { regionKey: 'stale-room' }, SPEC, progress),
    ).rejects.toBeInstanceOf(StaleHandle)
    expect(await archives(key)).toMatchObject([{ status: 'deleted', object_key: null }])
    expect(stored(key)).toEqual([])
  })

  test('a server that ran before a move keeps its room where it lands until it starts there', async () => {
    const { key, handle, to } = await moving('claimed-move')
    const from = decodeHandle(handle).node ?? ''
    const moved = await runtime.relocate(handle, { regionKey: 'claimed-move' }, SPEC, progress)
    expect(decodeHandle(moved).node).toBe(to)
    expect((await ledger(to))?.running.memoryMb).toBe(1024)
    // Room for a server needing all of that node isn't there now, so the move's start still is.
    await drain(db, from, 'test')
    await expect(
      runtime.ensureProvisioned(
        server(),
        { regionKey: 'claimed-move' },
        { ...SPEC, resources: { memoryMb: 4096 } },
        progress,
      ),
    ).rejects.toBeInstanceOf(RuntimeFull)
    await runtime.start(moved)
    expect(nodes.copies.get(`${to}/${key}`)?.state).toBe('running')
  })

  test('a server that was stopped moves without claiming room to run', async () => {
    const { handle, to } = await moving('stopped-move')
    await runtime.stop(handle)
    await runtime.relocate(handle, { regionKey: 'stopped-move' }, SPEC, progress)
    expect((await ledger(to))?.running.memoryMb).toBe(0)
  })

  test('a first placement its node never got is undone once nothing can still be making it', async () => {
    const [n] = (await region('never-made', 1)) as [EnrolledNode]
    const key = server()
    nodes.once(/^PUT /, async () => {
      throw new NodeUnreachable('timeout', 'no answer within 1200000 ms')
    })
    await expect(
      runtime.ensureProvisioned(key, { regionKey: 'never-made' }, SPEC, progress),
    ).rejects.toBeInstanceOf(NodeUnreachable)
    expect((await ledger(n.id))?.running.memoryMb).toBe(1024)
    // Recent: an attempt may still be making it.
    await runtime.upkeep()
    expect(await rows(db, sql`SELECT 1 FROM fleet_placements WHERE workload = ${key}`)).toHaveLength(1)
    // Old, but its node has been silent since: it may hold the copy, and hasn't said.
    await db.execute(
      sql`UPDATE fleet_placements SET updated_at = now() - interval '31 minutes' WHERE workload = ${key}`,
    )
    await db.execute(
      sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - interval '31 minutes' WHERE id = ${n.id}`,
    )
    await runtime.upkeep()
    expect(await rows(db, sql`SELECT 1 FROM fleet_placements WHERE workload = ${key}`)).toHaveLength(1)
    // Its node beats again without it: nothing is making it.
    await pulse(n)
    await runtime.upkeep()
    expect(await rows(db, sql`SELECT 1 FROM fleet_placements WHERE workload = ${key}`)).toEqual([])
    expect((await ledger(n.id))?.running.memoryMb).toBe(0)
    expect((await events('placement.abandoned', key)).map((e) => e.data)).toEqual([
      { code: 'never_made', minutes: 30 },
    ])
    const [history] = await rows<{ end_reason: string }>(
      db,
      sql`SELECT end_reason FROM fleet_placement_history WHERE workload = ${key}`,
    )
    expect(history?.end_reason).toBe('abandoned')
    // The next start places it afresh, under a new epoch.
    const placed = await runtime.ensureProvisioned(key, { regionKey: 'never-made' }, SPEC, progress)
    expect(decodeHandle(placed).epoch).toBe(2)
  })

  test('a first placement whose node reports its copy is finished, never abandoned', async () => {
    const [n] = (await region('made-late', 1)) as [EnrolledNode]
    const key = server()
    // The copy was made, but the answer was lost.
    nodes.once(/^PUT /, async () => {
      nodes.copies.set(`${n.id}/${key}`, { epoch: 1, state: 'created', ports: { game: 31_000 } })
      throw new NodeUnreachable('reset', 'connection reset')
    })
    await expect(
      runtime.ensureProvisioned(key, { regionKey: 'made-late' }, SPEC, progress),
    ).rejects.toBeInstanceOf(NodeUnreachable)
    await db.execute(
      sql`UPDATE fleet_placements SET updated_at = now() - interval '31 minutes' WHERE workload = ${key}`,
    )
    await pulse(n)
    await runtime.upkeep()
    const [placement] = await rows<{ state: string }>(
      db,
      sql`SELECT state FROM fleet_placements WHERE workload = ${key}`,
    )
    expect(placement?.state).toBe('placed')
    expect(await events('placement.abandoned', key)).toEqual([])
  })

  test("a deleted snapshot's copy in the store goes once the store answers again", async () => {
    const [n] = (await region('store-down', 1)) as [EnrolledNode]
    const key = server()
    const objectKey = `fleet/test/${key}/copy.tar.gz`
    const snap = await snapshotOn(n.id, key, objectKey)
    store.failing = true
    await runtime.deleteSnapshot(snap.handle)
    // The node's copy went; the store's is still there, and its key kept.
    expect(await archives(key)).toMatchObject([{ status: 'deleted', object_key: objectKey }])
    expect((await archives(key))[0]?.local_deleted_at).not.toBeNull()
    expect(stored(key)).toEqual([objectKey])
    await runtime.upkeep()
    expect(stored(key)).toEqual([objectKey])
    store.failing = false
    await runtime.upkeep()
    expect(stored(key)).toEqual([])
    expect(await archives(key)).toMatchObject([{ status: 'deleted', object_key: null }])
  })

  test("a deleted snapshot's copy on a lost node is deleted once that node beats again", async () => {
    const [n] = (await region('lost-copy', 1)) as [EnrolledNode]
    const key = server()
    const snap = await snapshotOn(n.id, key, null)
    const deletes = () =>
      nodes.calls.filter((c) => c === `${n.id} DELETE /v1/workloads/${key}/snapshots/${snap.id}`)
    await db.execute(
      sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - interval '10 minutes' WHERE id = ${n.id}`,
    )
    await confirmLost(db, THRESHOLDS, n.id, { fencedBy: 'test', reason: 'test', by: 'test' })
    await runtime.deleteSnapshot(snap.handle)
    expect(deletes()).toEqual([])
    expect((await archives(key))[0]?.local_deleted_at).toBeNull()
    // Still silent: nothing to ask.
    await runtime.upkeep()
    expect(deletes()).toHaveLength(0)
    // It beats again, still lost: its copy of the deleted snapshot goes, and nothing else.
    await pulse(n)
    await runtime.upkeep()
    expect(deletes()).toHaveLength(1)
    expect((await archives(key))[0]?.local_deleted_at).not.toBeNull()
  })

  /** A world of `mib` MiB that a node packs, sending at most 1 MiB in one PUT, for `run`'s length. */
  async function packing<T>(mib: number, run: (world: Buffer) => Promise<T>): Promise<T> {
    const world = Buffer.alloc(mib * 1024 ** 2, mib)
    nodes.archive = world
    nodes.maxPut = 1024 ** 2
    try {
      return await run(world)
    } finally {
      nodes.archive = Buffer.from('world')
      nodes.maxPut = Number.POSITIVE_INFINITY
    }
  }
  const uploadsOf = (snapshot: string) =>
    nodes.calls.filter((c) => c.endsWith(`/snapshots/${snapshot}/upload`))

  test('a snapshot larger than one PUT carries goes up in parts, and the store holds it whole', async () => {
    const [n] = (await region('parts', 1)) as [EnrolledNode]
    const key = server()
    const snap = await snapshotOn(n.id, key, null)
    await packing(20, async (world) => {
      const completed = store.completed
      expect(await runtime.upload(snap.id)).toBe('ready')
      // Asked for one PUT, which it refused saying how large, then in parts for that.
      expect(uploadsOf(snap.id)).toHaveLength(2)
      expect(store.completed).toBe(completed + 1)
      const [row] = await archives(key)
      expect(row?.status).toBe('ready')
      expect(store.objects.get(row?.object_key ?? '')).toEqual(world)
    })
  })

  test('a snapshot known to be large is offered parts at once; one that packs small drops them', async () => {
    const [n] = (await region('parts-known', 1)) as [EnrolledNode]
    const key = server()
    const before = store.maxPutBytes
    store.maxPutBytes = 1024 ** 2
    try {
      await packing(20, async () => {
        const snap = await snapshotOn(n.id, key, null)
        await db.execute(sql`UPDATE fleet_archives SET size_bytes = ${20 * 1024 ** 2} WHERE id = ${snap.id}`)
        expect(await runtime.upload(snap.id)).toBe('ready')
        expect(uploadsOf(snap.id)).toHaveLength(1)
      })
      // Expected large, it packed small: one PUT, and the upload in parts begun for it goes.
      const small = await snapshotOn(n.id, key, null)
      await db.execute(sql`UPDATE fleet_archives SET size_bytes = ${20 * 1024 ** 2} WHERE id = ${small.id}`)
      const aborted = store.aborted
      expect(await runtime.upload(small.id)).toBe('ready')
      expect(store.aborted).toBe(aborted + 1)
    } finally {
      store.maxPutBytes = before
    }
  })

  test('a copy larger than one PUT carries is exported from the store in parts, read by its ranges', async () => {
    const [n] = (await region('parts-copy', 1)) as [EnrolledNode]
    const key = server()
    const objectKey = `fleet/test/${key}/copy.tar.gz`
    const snap = await snapshotOn(n.id, key, objectKey)
    const world = randomBytes(20 * 1024 ** 2)
    const sha256 = createHash('sha256').update(world).digest('hex')
    store.objects.set(objectKey, world)
    // Its node no longer holds it: the export comes from the store.
    await db.execute(
      sql`UPDATE fleet_archives SET local_deleted_at = now(), sha256 = ${sha256} WHERE id = ${snap.id}`,
    )
    const before = store.maxPutBytes
    store.maxPutBytes = 1024 ** 2
    try {
      const sent = await runtime.exportSnapshot(snap.handle, await store.archiveTarget('elsewhere/copy'))
      expect(sent).toEqual({ sizeBytes: world.length, sha256 })
      expect(store.objects.get('elsewhere/copy')).toEqual(world)
      expect(uploadsOf(snap.id)).toEqual([])
    } finally {
      store.maxPutBytes = before
    }
  }, 30_000)

  test('a move of a world larger than one PUT carries goes through the store in parts', async () => {
    const { key, handle, to } = await moving('parts-move')
    const completed = store.completed
    await packing(20, async () => {
      const moved = await runtime.relocate(handle, { regionKey: 'parts-move' }, SPEC, progress)
      expect(decodeHandle(moved).node).toBe(to)
    })
    expect(store.completed).toBe(completed + 1)
    expect((await events('snapshot.upload_failed', key)).filter((e) => e.data.purpose === 'move')).toEqual([])
  })

  test('a snapshot too big for one upload is failed for good, saying why', async () => {
    const [n] = (await region('too-big', 1, ONE_PUT_ONLY)) as [EnrolledNode]
    const key = server()
    const snap = await snapshotOn(n.id, key, null)
    nodes.once(/\/upload$/, async () => {
      throw TOO_LARGE()
    })
    expect(await runtime.upload(snap.id)).toBe('failed')
    const reason =
      'Too big to upload: its archive is 6.0 GB, and one upload to the archive store carries at most 5.0 GB'
    expect(await archives(key)).toMatchObject([{ status: 'failed', error: reason }])
    expect((await events('snapshot.upload_failed', key)).map((e) => e.data)).toEqual([
      {
        id: snap.id,
        error: reason,
        code: 'archive_too_large',
        sizeBytes: 6 * 1024 ** 3,
        limitBytes: 5 * 1024 ** 3,
      },
    ])
    // Not sent again: the copy stays on its node, where a restore there still finds it.
    await runtime.uploadPending()
    expect(nodes.calls.filter((c) => c.endsWith(`/snapshots/${snap.id}/upload`))).toHaveLength(1)
    expect(await runtime.goneSnapshots([snap.handle])).toEqual(new Set())
    // Packed for the application (a download, the copy a world rests in), it is refused as what
    // no retry changes.
    nodes.once(/\/upload$/, async () => {
      throw TOO_LARGE()
    })
    const packed = runtime.exportSnapshot(snap.handle, await store.archiveTarget('elsewhere/archive'))
    await expect(packed).rejects.toBeInstanceOf(RuntimeUnsupported)
    await expect(packed).rejects.toThrow('one upload to the archive store carries at most 5.0 GB')
  })

  test('an export waits out the snapshot’s own upload off its node, then asks the node', async () => {
    const [n] = (await region('busy-export', 1)) as [EnrolledNode]
    const snap = await snapshotOn(n.id, server(), null)
    // Its upload to the store, begun when it was taken, is still under way, for a second more.
    await db.execute(
      sql`UPDATE fleet_archives SET status = 'uploading', upload_started_at = now() WHERE id = ${snap.id}`,
    )
    const began = Date.now()
    // A query runs when it is awaited, so this one is.
    const finished = setTimeout(async () => {
      await db.execute(sql`UPDATE fleet_archives SET status = 'local' WHERE id = ${snap.id}`)
    }, 1_000)
    try {
      await runtime.exportSnapshot(snap.handle, await store.archiveTarget('busy/waited'))
    } finally {
      clearTimeout(finished)
    }
    expect(Date.now() - began).toBeGreaterThanOrEqual(1_000)
    expect(uploadsOf(snap.id)).toHaveLength(1)
    expect(store.objects.get('busy/waited')?.toString()).toBe('world')
  }, 15_000)

  test('an export a node refuses as busy with that snapshot asks again, once it is free', async () => {
    const [n] = (await region('busy-node', 1)) as [EnrolledNode]
    const snap = await snapshotOn(n.id, server(), null)
    nodes.once(/\/upload$/, async () => {
      throw new NodeRefused(409, 'snapshot_busy', 'this snapshot is being uploaded already', undefined)
    })
    await runtime.exportSnapshot(snap.handle, await store.archiveTarget('busy/again'))
    expect(uploadsOf(snap.id)).toHaveLength(2)
    expect(store.objects.get('busy/again')?.toString()).toBe('world')
  }, 15_000)

  test('an upload off a node busy sending that snapshot elsewhere is not a failed try', async () => {
    const [n] = (await region('busy-upload', 1)) as [EnrolledNode]
    const key = server()
    const snap = await snapshotOn(n.id, key, null)
    nodes.once(/\/upload$/, async () => {
      throw new NodeRefused(409, 'snapshot_busy', 'this snapshot is being uploaded already', undefined)
    })
    expect(await runtime.upload(snap.id)).toBe('skipped')
    const attempts = await rows<{ status: string; attempts: number }>(
      db,
      sql`SELECT status, attempts FROM fleet_archives WHERE id = ${snap.id}`,
    )
    expect(attempts).toEqual([{ status: 'local', attempts: 0 }])
    expect(await runtime.upload(snap.id)).toBe('ready')
    expect(await events('snapshot.upload_failed', key)).toEqual([])
  })

  test('a move of a world too big for one upload is declined, and the server runs where it was', async () => {
    const { key, handle, to } = await moving('too-big-move', ONE_PUT_ONLY)
    const starts = () => nodes.calls.filter((c) => c.endsWith(`/v1/workloads/${key}/start`)).length
    const startedBefore = starts()
    nodes.once(/^POST .*\/export$/, async () => {
      throw TOO_LARGE()
    })
    const declined = runtime.relocate(handle, { regionKey: 'too-big-move' }, SPEC, progress)
    await expect(declined).rejects.toBeInstanceOf(RuntimeUnsupported)
    await expect(declined).rejects.toThrow(
      'its archive is 6.0 GB, and one upload to the archive store carries at most 5.0 GB',
    )
    expect(await archives(key)).toMatchObject([{ status: 'failed' }])
    expect((await events('snapshot.upload_failed', key)).map((e) => e.data.purpose)).toEqual(['move'])
    // It stopped to be copied, and runs again where it was, under the same epoch.
    expect(starts()).toBe(startedBefore + 1)
    const [placement] = await rows<{ node_id: string; epoch: string; move_requested_at: string | null }>(
      db,
      sql`SELECT node_id, epoch, move_requested_at FROM fleet_placements WHERE workload = ${key}`,
    )
    expect(placement?.node_id).toBe(decodeHandle(handle).node ?? '')
    expect(Number(placement?.epoch)).toBe(decodeHandle(handle).epoch)
    // The operator's request can never be met, so it is dropped, saying why.
    expect(placement?.move_requested_at).toBeNull()
    expect(await events('placement.move_declined', key)).toHaveLength(1)

    // Asked again, it is declined before anything stops or is copied.
    await runtime.requestMove(key, to, 'operator:test')
    const since = nodes.calls.length
    await expect(
      runtime.relocate(handle, { regionKey: 'too-big-move' }, SPEC, progress),
    ).rejects.toBeInstanceOf(RuntimeUnsupported)
    expect(nodes.calls.slice(since).filter((c) => /\/(stop|export)$/.test(c))).toEqual([])
    // A move nobody asked for by name (a drain) is declined the same way, and recorded too.
    await drain(db, decodeHandle(handle).node ?? '', 'test')
    await expect(
      runtime.relocate(handle, { regionKey: 'too-big-move' }, SPEC, progress),
    ).rejects.toBeInstanceOf(RuntimeUnsupported)
    const recorded = (await events('placement.move_declined', key)).map((e) => e.data.requested)
    expect(recorded.sort()).toEqual([false, true, true])
  })

  test('a start whose world is too big to move to a node with room waits for room where it is', async () => {
    const [a, b] = await region('too-big-room', 2, ONE_PUT_ONLY)
    const key = server()
    const handle = await runtime.ensureProvisioned(key, { regionKey: 'too-big-room' }, SPEC, progress)
    const home = decodeHandle(handle).node === a?.id ? (a as EnrolledNode) : (b as EnrolledNode)
    const away = home === a ? (b as EnrolledNode) : (a as EnrolledNode)
    await runtime.stop(handle)
    // Another server takes its node's room while it sleeps.
    await drain(db, away.id, 'test')
    await runtime.ensureProvisioned(
      server(),
      { regionKey: 'too-big-room' },
      { ...SPEC, resources: { memoryMb: 3584 } },
      progress,
    )
    await undrain(db, away.id, 'test')
    nodes.once(/^POST .*\/export$/, async () => {
      throw TOO_LARGE()
    })
    const waits = runtime.ensureProvisioned(key, { regionKey: 'too-big-room' }, SPEC, progress)
    await expect(waits).rejects.toBeInstanceOf(RuntimeFull)
    await expect(waits).rejects.toThrow("it can't move to another node: its archive is 6.0 GB")
    // The next start knows without copying the world again.
    const since = nodes.calls.length
    await expect(
      runtime.ensureProvisioned(key, { regionKey: 'too-big-room' }, SPEC, progress),
    ).rejects.toBeInstanceOf(RuntimeFull)
    expect(nodes.calls.slice(since).filter((c) => c.endsWith('/export'))).toEqual([])
  })
})
