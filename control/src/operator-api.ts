// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What an operator can ask of the fleet (docs/fleet-operations.md), on the internal listener, with
 * the operator token: which nodes there are and how they are, enrollment tokens, a node's labels,
 * draining, a lost host confirmed, a node retired, a server moved, and what happened.
 * `scripts/fleet.ts` is its command line. Nothing here starts or stops a server: a move goes through
 * the application's own relocation, which stops, snapshots and restarts around it.
 */

import { timingSafeEqual } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { Hono } from 'hono'
import type { Db } from './database.ts'
import { DAY_S, HOUR_S } from './durations.ts'
import type { Thresholds } from './health.ts'
import { type JoinPoint, joinCommand, joinToken } from './join-token.ts'
import {
  clearQuarantine,
  confirmLost,
  createToken,
  drain,
  getNode,
  nodeSummaries,
  Refused,
  reinstate,
  retire,
  setLabels,
  undrain,
} from './registry.ts'
import type { FleetRuntime } from './runtime-engine.ts'
import { rows } from './sql.ts'

export interface OperatorApiOptions {
  db: Db
  runtime: FleetRuntime
  thresholds: Thresholds
  token: string
  /** Asks the application to relocate a server, as its relocation sweep would. */
  relocate: (sandboxId: string) => Promise<void>
  /** The node endpoint's numbers, where this process serves it. */
  endpointStats?: () => object
  /** Where new hosts join, for the tokens and the line an operator pastes; main.node.ts gives it. */
  join?: JoinPoint
  /** The fleet regions there are, what PRYX_REGION_MAP maps to; a token is for one of them. */
  regions?: readonly string[]
}

export const OPERATOR_BASE = '/fleet/v1'

const NODE_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

export function createOperatorApi(options: OperatorApiOptions): Hono {
  const { db, thresholds } = options
  const app = new Hono()
  const expected = Buffer.from(`Bearer ${options.token}`)
  const by = (c: { req: { header: (name: string) => string | undefined } }) =>
    `operator:${(c.req.header('x-operator') ?? 'unnamed').slice(0, 64)}`

  app.use(`${OPERATOR_BASE}/*`, async (c, next) => {
    const given = Buffer.from(c.req.header('authorization') ?? '')
    if (given.length !== expected.length || !timingSafeEqual(given, expected))
      return c.json({ error: 'unauthorized' }, 401)
    await next()
  })
  app.onError((error, c) => {
    if (error instanceof Refused)
      return c.json({ error: { code: error.code, message: error.message } }, error.status as 400)
    return c.json({ error: { code: 'failed', message: (error as Error).message } }, 500)
  })
  const nodeId = (value: string) => {
    if (!NODE_ID.test(value)) throw new Refused(400, 'invalid_node', 'a node is named by its id')
    return value
  }
  const json = async (c: { req: { json: () => Promise<unknown> } }) =>
    ((await c.req.json().catch(() => ({}))) ?? {}) as Record<string, unknown>

  /** Every node, as an operator reads it: health, lifecycle, capacity and what is placed there. */
  app.get(`${OPERATOR_BASE}/nodes`, async (c) => {
    const nodes = await nodeSummaries(db, thresholds, {
      includeRetired: c.req.query('all') === 'true',
    })
    return c.json({ nodes: nodes.map(nodeView) })
  })

  app.get(`${OPERATOR_BASE}/nodes/:id`, async (c) => {
    const id = nodeId(c.req.param('id'))
    const node = (await nodeSummaries(db, thresholds, { includeRetired: true })).find((n) => n.id === id)
    if (node === undefined) throw new Refused(404, 'unknown_node', 'no such node')
    const placements = await rows(
      db,
      sql`SELECT p.workload, p.epoch, p.state, p.memory_mb, p.desired_power, p.completing, p.move_requested_at,
                 o.state AS observed, o.observed_at, o.state_changed_at
          FROM fleet_placements p
          LEFT JOIN fleet_observations o ON o.node_id = p.node_id AND o.workload = p.workload
          WHERE p.node_id = ${id} ORDER BY p.workload`,
    )
    const strays = await rows(
      db,
      sql`SELECT o.workload, o.epoch, o.superseded_by, o.state FROM fleet_observations o
          LEFT JOIN fleet_placements p ON p.workload = o.workload AND p.node_id = o.node_id
          WHERE o.node_id = ${id} AND p.workload IS NULL`,
    )
    const events = await rows(
      db,
      sql`SELECT at, kind, workload, epoch, data FROM fleet_events WHERE node_id = ${id} ORDER BY at DESC LIMIT 50`,
    )
    return c.json({
      node: nodeView(node),
      placements,
      copiesNotPlacedHere: strays,
      events,
    })
  })

  app.post(`${OPERATOR_BASE}/tokens`, async (c) => c.json(await mintToken(options, await json(c), by(c))))

  app.post(`${OPERATOR_BASE}/nodes/:id/drain`, async (c) =>
    c.json(nodeView(await drainView(c.req.param('id'), 'drain', by(c)))),
  )
  app.post(`${OPERATOR_BASE}/nodes/:id/undrain`, async (c) =>
    c.json(nodeView(await drainView(c.req.param('id'), 'undrain', by(c)))),
  )

  async function drainView(id: string, what: 'drain' | 'undrain', who: string) {
    await (what === 'drain' ? drain : undrain)(db, nodeId(id), who)
    const node = (await nodeSummaries(db, thresholds)).find((n) => n.id === id)
    if (node === undefined) throw new Refused(404, 'unknown_node', 'no such node')
    await options.runtime.refreshRegistry()
    return node
  }

  /**
   * The operator's statement that a host is gone, and how it was stopped. Its sandboxes are rebuilt
   * from their newest backups where there is room, after the application's grace.
   */
  app.post(`${OPERATOR_BASE}/nodes/:id/lost`, async (c) => {
    const body = await json(c)
    const result = await confirmLost(db, thresholds, nodeId(c.req.param('id')), {
      fencedBy: String(body.fencedBy ?? ''),
      reason: String(body.reason ?? ''),
      by: by(c),
      force: body.force === true,
    })
    await options.runtime.refreshRegistry()
    return c.json(result)
  })

  app.post(`${OPERATOR_BASE}/nodes/:id/reinstate`, async (c) => {
    const result = await reinstate(db, nodeId(c.req.param('id')), by(c))
    await options.runtime.refreshRegistry()
    return c.json(result)
  })

  app.post(`${OPERATOR_BASE}/nodes/:id/retire`, async (c) => {
    await retire(db, nodeId(c.req.param('id')), by(c))
    await options.runtime.refreshRegistry()
    return c.json({ retired: true })
  })

  app.post(`${OPERATOR_BASE}/nodes/:id/clear-quarantine`, async (c) => {
    await clearQuarantine(db, nodeId(c.req.param('id')), by(c))
    return c.json({ cleared: true })
  })

  /** A node's labels set or removed: its monthly price (`monthly_cost_cents`) among them. */
  app.post(`${OPERATOR_BASE}/nodes/:id/labels`, async (c) => {
    const id = nodeId(c.req.param('id'))
    const body = await json(c)
    const set = (body.set ?? {}) as Record<string, string>
    const remove = (body.remove ?? []) as string[]
    if (
      typeof set !== 'object' ||
      Array.isArray(set) ||
      Object.entries(set).some(([k, v]) => k === '' || typeof v !== 'string') ||
      !Array.isArray(remove) ||
      remove.some((k) => typeof k !== 'string' || k === '')
    )
      throw new Refused(400, 'invalid_request', 'labels are names and text')
    await setLabels(db, id, { set, remove, by: by(c) })
    // Prices are read from the registry this process keeps; the others reread it within seconds.
    await options.runtime.refreshRegistry()
    const node = (await nodeSummaries(db, thresholds, { includeRetired: true })).find((n) => n.id === id)
    if (node === undefined) throw new Refused(404, 'unknown_node', 'no such node')
    return c.json(nodeView(node))
  })

  app.get(`${OPERATOR_BASE}/placements`, async (c) => {
    const node = c.req.query('node')
    const found = await rows(
      db,
      sql`SELECT p.workload, p.node_id, n.name AS node, p.epoch, p.state, p.region_key, p.memory_mb, p.disk_gb,
                 p.desired_power, p.completing, p.move_requested_at, p.updated_at,
                 o.state AS observed, o.state_changed_at,
                 (SELECT max(a.captured_at) FROM fleet_archives a
                   WHERE a.workload = p.workload AND a.purpose = 'snapshot' AND a.status = 'ready') AS newest_stored_copy
          FROM fleet_placements p
          LEFT JOIN fleet_nodes n ON n.id = p.node_id
          LEFT JOIN fleet_observations o ON o.node_id = p.node_id AND o.workload = p.workload
          WHERE ${node === undefined ? sql`TRUE` : sql`p.node_id = ${nodeId(node)}`}
          ORDER BY n.name NULLS LAST, p.workload`,
    )
    return c.json({ placements: found })
  })

  /** One server's placement, its history of homes, its copies, and every node's report of it. */
  app.get(`${OPERATOR_BASE}/placements/:server`, async (c) => {
    const server = c.req.param('server')
    const [placement] = await rows(db, sql`SELECT * FROM fleet_placements WHERE workload = ${server}`)
    const history = await rows(
      db,
      sql`SELECT * FROM fleet_placement_history WHERE workload = ${server} ORDER BY epoch`,
    )
    const archives = await rows(
      db,
      sql`SELECT id, purpose, epoch, node_id, status, size_bytes, sha256, consistency, captured_at, uploaded_at,
                 attempts, error, local_deleted_at IS NULL AND local_id IS NOT NULL AS on_node
          FROM fleet_archives WHERE workload = ${server} AND status <> 'deleted' ORDER BY captured_at DESC LIMIT 50`,
    )
    const observations = await rows(db, sql`SELECT * FROM fleet_observations WHERE workload = ${server}`)
    if (placement === undefined && observations.length === 0)
      throw new Refused(404, 'unknown_server', 'the fleet holds nothing for that server')
    return c.json({
      placement: placement ?? null,
      history,
      archives,
      observations,
    })
  })

  /** Moves one server off its node, to `to` if named, through the application's relocation. */
  app.post(`${OPERATOR_BASE}/placements/:server/move`, async (c) => {
    const server = c.req.param('server')
    const body = await json(c)
    const to = body.to === undefined || body.to === null ? null : nodeId(String(body.to))
    if (to !== null && (await getNode(db, to)) === null)
      throw new Refused(404, 'unknown_node', 'no such node')
    await options.runtime.requestMove(server, to, by(c))
    await options.relocate(server)
    return c.json({ requested: true, to })
  })

  app.get(`${OPERATOR_BASE}/events`, async (c) => {
    const node = c.req.query('node')
    const server = c.req.query('server')
    const limit = Math.min(Math.max(Number(c.req.query('limit') ?? 100), 1), 1000)
    const found = await rows(
      db,
      sql`SELECT at, kind, node_id, workload, epoch, data FROM fleet_events
          WHERE ${node === undefined ? sql`TRUE` : sql`node_id = ${nodeId(node)}`}
            AND ${server === undefined ? sql`TRUE` : sql`workload = ${server}`}
          ORDER BY at DESC LIMIT ${limit}`,
    )
    return c.json({ events: found })
  })

  /** The whole fleet in numbers: nodes by health and lifecycle, room, and how fresh backups are. */
  app.get(`${OPERATOR_BASE}/summary`, async (c) => {
    const nodes = await nodeSummaries(db, thresholds)
    const [backups] = await rows<{
      sandboxes: string
      without_stored_copy: string
      oldest_newest_copy: Date | null
    }>(
      db,
      sql`WITH newest AS (
            SELECT p.workload, max(a.captured_at) AS at FROM fleet_placements p
            LEFT JOIN fleet_archives a ON a.workload = p.workload AND a.purpose = 'snapshot' AND a.status = 'ready'
            WHERE p.state IN ('placed', 'displaced') GROUP BY p.workload)
          SELECT count(*) AS sandboxes, count(*) FILTER (WHERE at IS NULL) AS without_stored_copy, min(at) AS oldest_newest_copy
          FROM newest`,
    )
    const pendingUploads = await rows(
      db,
      sql`SELECT status, count(*) AS copies FROM fleet_archives WHERE status IN ('local', 'uploading', 'failed') GROUP BY status`,
    )
    const by = <K extends string>(key: (n: (typeof nodes)[number]) => K) =>
      nodes.reduce<Record<string, number>>((acc, n) => {
        acc[key(n)] = (acc[key(n)] ?? 0) + 1
        return acc
      }, {})
    const sum = (f: (n: (typeof nodes)[number]) => number) => nodes.reduce((acc, n) => acc + f(n), 0)
    return c.json({
      nodes: {
        total: nodes.length,
        byHealth: by((n) => n.derivedHealth),
        byLifecycle: by((n) => n.lifecycle),
      },
      memoryMb: {
        allocatable: sum((n) => n.capacity.allocatableMemoryMb ?? 0),
        allocated: sum((n) => n.allocated.memoryMb),
        running: sum((n) => n.running.memoryMb),
      },
      sandboxes: {
        placed: sum((n) => n.allocated.workloads),
        running: sum((n) => n.running.workloads),
      },
      backups: {
        sandboxes: Number(backups?.sandboxes ?? 0),
        withoutStoredCopy: Number(backups?.without_stored_copy ?? 0),
        oldestNewestCopy: backups?.oldest_newest_copy ?? null,
        notYetStored: pendingUploads,
      },
      endpoint: options.endpointStats?.() ?? null,
    })
  })

  return app
}

/**
 * A one-time token, in the two forms an operator uses: `token`, the bare secret, and
 * `joinToken`, the `bk1.` form that also names where to join (join-token.ts), and with `joinCommand`,
 * the line pasted on the host. A token that re-enrolls a node names it in `joinToken`, and its line
 * is pasted on that node's host (docs/fleet-operations.md §8).
 */
async function mintToken(options: OperatorApiOptions, body: Record<string, unknown>, createdBy: string) {
  const regionKey = String(body.region ?? '')
  if (!regionKey && body.node === undefined)
    throw new Refused(400, 'invalid_request', 'say which region the node joins')
  const labels = (body.labels ?? {}) as Record<string, string>
  if (typeof labels !== 'object' || Object.values(labels).some((v) => typeof v !== 'string'))
    throw new Refused(400, 'invalid_request', 'labels are names and text')
  const ttlSeconds = Math.min(Math.max(Number(body.ttlSeconds ?? HOUR_S), 60), 7 * DAY_S)
  const node = body.node === undefined || body.node === null ? undefined : String(body.node)
  if (node !== undefined && !NODE_ID.test(node))
    throw new Refused(400, 'invalid_node', 'a node is named by its id')
  const made = await createToken(options.db, {
    regionKey,
    labels,
    ttlSeconds,
    createdBy,
    ...(options.regions ? { regions: options.regions } : {}),
    ...(node ? { node } : {}),
  })
  if (options.join === undefined) return made
  const pasted = joinToken(options.join, made.token, node)
  return {
    ...made,
    joinToken: pasted,
    joinCommand: joinCommand(options.join, pasted),
  }
}

/**
 * A node as operators read it: what it is, how it is, and the room it has. In the ledger,
 * `allocated` is every server placed on it, running or asleep, and `running` those that run or are
 * starting: a start needs room beside these (docs/fleet.md, "Admission").
 */
function nodeView(node: Awaited<ReturnType<typeof nodeSummaries>>[number]) {
  const c = node.capacity
  return {
    id: node.id,
    name: node.name,
    region: node.region_key,
    lifecycle: node.lifecycle,
    health: node.derivedHealth,
    quarantined: node.duplicate_seen_at !== null,
    lastHeartbeatSecondsAgo: node.age === null ? null : Math.round(node.age),
    version: node.daemon_version,
    addresses: {
      api: node.api_address,
      edge: node.edge_host,
      control: node.control_host,
    },
    runtimeUp: node.runtime_up,
    certificateExpiresAt: node.cert_expires_at,
    memoryMb: {
      total: c.memoryTotalMb ?? null,
      reservedForHost: c.reservedMemoryMb ?? null,
      allocatable: c.allocatableMemoryMb ?? null,
      allocated: node.allocated.memoryMb,
      running: node.running.memoryMb,
      runningServers: c.runningMemoryMb ?? null,
      usedBytes: c.usedMemoryBytes ?? null,
    },
    cpu: {
      cores: c.cpus ?? null,
      reservedForHostMillis: c.reservedCpuMillis ?? null,
      allocatedMillis: node.allocated.cpuMillis,
      runningMillis: node.running.cpuMillis,
      usedCores: c.usedCpuCores ?? null,
      loadAverage: c.loadAverage ?? null,
    },
    disk: {
      totalBytes: c.diskTotalBytes ?? null,
      availableBytes: c.diskAvailableBytes ?? null,
      reservedGb: node.allocated.diskGb,
      snapshotBytes: c.snapshotBytes ?? null,
      reflink: c.reflink ?? null,
    },
    ports: { total: c.portsTotal ?? null, allocated: c.portsAllocated ?? null },
    sandboxes: node.allocated.workloads,
    serversRunning: node.running.workloads,
    issues: node.issues,
    lastProbe: node.last_probe,
    labels: node.labels,
    lostAt: node.lost_at,
    lostReason: node.lost_reason,
    fencedBy: node.fenced_by,
  }
}
