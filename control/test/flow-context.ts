// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Shared database and node setup for fleet flow assertions. */

import { afterAll, beforeAll } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { FleetStore } from '../src/archive-transport.ts'
import type { FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { decodeHandle, encodeSnapshot } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import type { NodeClient } from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { heartbeat, nodeSummaries } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import {
  type ProgressSink,
  type RuntimeKey,
  type RuntimeSpec,
  runtimeKey,
  type SnapshotHandle,
} from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import { FlakyStore, Nodes } from './flow-nodes.ts'
import {
  beat,
  capacity,
  type EnrolledNode,
  enrollNode,
  freshDatabase,
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

export function flowContext() {
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
  return {
    get SPEC() {
      return SPEC
    },
    get FEATURES() {
      return FEATURES
    },
    get CONFIRM() {
      return CONFIRM
    },
    get db() {
      return db
    },
    get drop() {
      return drop
    },
    get ca() {
      return ca
    },
    get store() {
      return store
    },
    get nodes() {
      return nodes
    },
    get runtime() {
      return runtime
    },
    get logs() {
      return logs
    },
    get seq() {
      return seq
    },
    get pulse() {
      return pulse
    },
    get region() {
      return region
    },
    get server() {
      return server
    },
    get nameOf() {
      return nameOf
    },
    get callsSince() {
      return callsSince
    },
    get kinds() {
      return kinds
    },
    get lastEvent() {
      return lastEvent
    },
    get eventData() {
      return eventData
    },
    get placement() {
      return placement
    },
    get history() {
      return history
    },
    get archives() {
      return archives
    },
    get running() {
      return running
    },
    get stored() {
      return stored
    },
    get recorder() {
      return recorder
    },
    get quiet() {
      return quiet
    },
    get snapshotOn() {
      return snapshotOn
    },
    get moving() {
      return moving
    },
  }
}
