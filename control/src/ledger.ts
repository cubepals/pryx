// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The placement ledger: where each server's data lives (`fleet_placements`), under which epoch,
 * and the history of its homes (`fleet_placement_history`). An epoch is only ever raised, by
 * compare-and-set in the transaction that gives a data its new home, and a handle for an older
 * one is refused (`StaleHandle`).
 *
 * Every statement this runtime writes to those tables is here, apart from the claim on a node's
 * room to run (`desired_power`, `power_changed_at` and the claimed memory), which is
 * `admission.ts`'s. Outside the runtime, the registry writes them too: `registry/heartbeat.ts`
 * adopts a node's newer epoch after a database restore, and `registry/lifecycle.ts` marks a lost
 * node's placements `displaced` and a reinstated one's `placed` again.
 */

import { sql } from 'drizzle-orm'

import type { Db, Queryable } from './database.ts'
import { decodeHandle, encodeHandle, type FleetRef } from './handle.ts'
import type { Considered, Lifecycle, PlacementRequest } from './placement.ts'
import { getNode, type NodeRow } from './registry.ts'
import type { RuntimeHandle } from './runtime-model.ts'
import { event, exec, jsonb, one, rows } from './sql.ts'
import type { WorkloadReport, WorkloadView } from './wire.ts'

/**
 * How long an operation may run before the job queue counts it stuck (infra/pg/jobs.ts): a first
 * placement nothing has touched for longer is worked on by nobody.
 */
const ABANDONED_MINUTES = 30

export interface PlacementRow {
  workload: string
  node_id: string | null
  epoch: string
  state: 'placing' | 'placed' | 'released' | 'displaced'
  region_key: string
  memory_mb: number
  cpu_millis: number
  disk_gb: number
  ports: Record<string, number>
  handle: string | null
  spec_digest: string | null
  sent_digest: string | null
  completing: 'restore' | 'move' | 'recover' | null
  restore_from: string | null
  desired_power: string
  move_requested_at: string | null
  move_to: string | null
  updated_at: string
}

/** The handle names a placement that has since been replaced. Its holder should read the new one. */
export class StaleHandle extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'StaleHandle'
  }
}

export const hostLost = () =>
  new Error(
    "This server's host was lost. It is rebuilt from its newest backup where there is room; nothing runs it meanwhile.",
  )

/** A placement with its node and its node's last report of it. */
export const OBSERVE_SQL = sql.raw(`
  SELECT p.*, n.name AS node_name, n.lifecycle, n.lost_at, n.health_since, n.runtime_up, n.reconciled,
         n.edge_host, n.control_host, EXTRACT(EPOCH FROM now() - n.last_heartbeat_at)::float8 AS age,
         o.state AS observed_state, o.epoch AS observed_epoch, o.report, o.state_changed_at
  FROM fleet_placements p
  LEFT JOIN fleet_nodes n ON n.id = p.node_id
  LEFT JOIN fleet_observations o ON o.node_id = p.node_id AND o.workload = p.workload`)

export interface ObservedRow extends PlacementRow {
  node_name: string | null
  lifecycle: Lifecycle | null
  lost_at: string | null
  health_since: string | null
  runtime_up: boolean | null
  reconciled: boolean | null
  edge_host: string | null
  control_host: string | null
  age: number | null
  observed_state: string | null
  observed_epoch: string | null
  report: WorkloadReport | null
  state_changed_at: string | null
}

export async function placementOf(
  q: Queryable,
  key: string,
  forUpdate = false,
): Promise<PlacementRow | null> {
  return one<PlacementRow>(
    q,
    sql`SELECT * FROM fleet_placements WHERE workload = ${key} ${sql.raw(forUpdate ? 'FOR UPDATE' : '')}`,
  )
}

/** Never reused, never lowered: above anything the database or any node has seen for the key. */
export async function nextEpoch(q: Queryable, key: string): Promise<number> {
  const row = await one<{ next: string }>(
    q,
    sql`SELECT GREATEST(
          (SELECT COALESCE(MAX(epoch), 0) FROM fleet_placement_history WHERE workload = ${key}),
          (SELECT COALESCE(MAX(epoch), 0) FROM fleet_placements WHERE workload = ${key}),
          (SELECT COALESCE(MAX(GREATEST(epoch, COALESCE(superseded_by, 0))), 0)
             FROM fleet_observations WHERE workload = ${key})
        ) + 1 AS next`,
  )
  return Number(row?.next ?? 1)
}

async function endHistory(q: Queryable, key: string, epoch: number, reason: string): Promise<void> {
  await q.execute(sql`
    UPDATE fleet_placement_history SET ended_at = now(), end_reason = ${reason}
    WHERE workload = ${key} AND epoch = ${epoch} AND ended_at IS NULL`)
}

/** The placement moves to a node under a new epoch, in the caller's transaction. */
export async function rehome(
  t: Queryable,
  row: PlacementRow,
  to: {
    node: string
    epoch: number
    region: string
    request: PlacementRequest
  },
  why: {
    ended: string
    started: 'restore' | 'move' | 'recover'
    restoreFrom: string | null
  },
): Promise<PlacementRow> {
  const moved = await one<PlacementRow>(
    t,
    sql`UPDATE fleet_placements SET node_id = ${to.node}, epoch = ${to.epoch}, state = 'placing',
          region_key = ${to.region}, memory_mb = ${to.request.memoryMb}, cpu_millis = ${to.request.cpuMillis},
          disk_gb = ${to.request.diskGb}, completing = ${why.started}, restore_from = ${why.restoreFrom},
          sent_digest = NULL, move_requested_at = NULL, move_to = NULL, updated_at = now()
        WHERE workload = ${row.workload} AND epoch = ${row.epoch} RETURNING *`,
  )
  if (moved === null) throw new StaleHandle('The placement changed while a new home was chosen for it')
  await endHistory(t, row.workload, Number(row.epoch), why.ended)
  await t.execute(sql`
    INSERT INTO fleet_placement_history (workload, epoch, node_id, reason)
    VALUES (${row.workload}, ${to.epoch}, ${to.node}, ${why.started})`)
  await event(t, 'placement.rehomed', {
    node: to.node,
    workload: row.workload,
    epoch: to.epoch,
    data: {
      fromNode: row.node_id,
      fromEpoch: Number(row.epoch),
      why: why.started,
    },
  })
  return moved
}

/**
 * A server's first placement, in the caller's transaction: `placing` on its node, with its room
 * to run claimed, since it starts as soon as it is made.
 */
export async function place(
  t: Queryable,
  first: {
    key: string
    node: string
    epoch: number
    region: string
    request: PlacementRequest
    policy: string
    considered: Considered[]
  },
): Promise<PlacementRow | null> {
  const { key, epoch, region, request } = first
  const inserted = await one<PlacementRow>(
    t,
    sql`INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb,
          desired_power, power_changed_at)
        VALUES (${key}, ${first.node}, ${epoch}, 'placing', ${region}, ${request.memoryMb},
                ${request.cpuMillis}, ${request.diskGb}, 'running', now())
        RETURNING *`,
  )
  await t.execute(sql`
    INSERT INTO fleet_placement_history (workload, epoch, node_id, reason)
    VALUES (${key}, ${epoch}, ${first.node}, 'placed')`)
  await event(t, 'placement.created', {
    node: first.node,
    workload: key,
    epoch,
    data: { policy: first.policy, considered: first.considered },
  })
  return inserted
}

/** A destroyed server's placement, in the caller's transaction: its epoch ended, its row gone. */
export async function remove(t: Queryable, key: string, epoch: number): Promise<void> {
  await endHistory(t, key, epoch, 'destroyed')
  await t.execute(sql`DELETE FROM fleet_placements WHERE workload = ${key}`)
}

export class Ledger {
  readonly #db: Db
  readonly #deployment: string

  constructor(options: { db: Db; deployment: string }) {
    this.#db = options.db
    this.#deployment = options.deployment
  }

  /** The node a placement names, which must still be in the fleet. */
  async nodeOf(id: string | null): Promise<NodeRow> {
    if (id === null) throw new Error('The placement names no node')
    const node = await getNode(this.#db, id)
    if (node === null) throw new Error(`No node ${id}`)
    if (node.lifecycle === 'lost') throw hostLost()
    if (node.lifecycle === 'retired') throw new Error(`Node ${node.name} is retired`)
    return node
  }

  /** The placement a handle names, which must still be the current one. */
  async current(handle: RuntimeHandle): Promise<{ ref: FleetRef; row: PlacementRow; node: NodeRow }> {
    const ref = decodeHandle(handle)
    const row = await placementOf(this.#db, ref.key)
    if (row === null) throw new Error(`Nothing is placed for ${ref.key}`)
    if (row.state === 'displaced') throw hostLost()
    if (row.state === 'released' || row.node_id === null)
      throw new Error("This server's data rests in the archive store.")
    if (Number(row.epoch) !== ref.epoch || row.node_id !== ref.node)
      throw new StaleHandle(`This handle is for epoch ${ref.epoch}; the server is at ${row.epoch}`)
    return { ref, row, node: await this.nodeOf(row.node_id) }
  }

  handleOf(
    row: Pick<ObservedRow, 'workload' | 'node_id' | 'epoch' | 'region_key' | 'ports' | 'handle'> &
      Partial<Pick<ObservedRow, 'node_name' | 'edge_host' | 'control_host' | 'state'>>,
    agent?: import('./wire.ts').AgentView | null,
  ): RuntimeHandle {
    if (agent === undefined && row.handle !== null && row.state !== 'placing') {
      const stored = decodeHandle(row.handle)
      if (stored.epoch === Number(row.epoch) && stored.node === row.node_id)
        return row.handle as RuntimeHandle
    }
    return encodeHandle({
      deployment: this.#deployment,
      key: row.workload,
      node: row.node_id,
      nodeName: row.node_name ?? null,
      epoch: Number(row.epoch),
      region: row.region_key,
      edgeHost: row.edge_host ?? null,
      controlHost: row.control_host ?? null,
      ports: row.ports,
      ...(agent ? { agent } : {}),
    })
  }

  /**
   * A placement with no node, as a released one: its data comes from the archive store when it is
   * restored, onto whichever node has room then. One already here is handed back as it is.
   */
  async adopt(
    key: string,
    region: string,
    request: PlacementRequest,
    ports: Record<string, number>,
  ): Promise<RuntimeHandle> {
    return this.#db.transaction(async (t) => {
      const row = await placementOf(t, key, true)
      if (row !== null) {
        if (row.state !== 'released') throw new Error(`The fleet holds ${key} already (${row.state})`)
        return encodeHandle({
          deployment: this.#deployment,
          key,
          node: null,
          nodeName: null,
          epoch: Number(row.epoch),
          region: row.region_key,
          edgeHost: null,
          controlHost: null,
          ports: { ...ports, ...(row.ports as Record<string, number>) },
        })
      }
      const epoch = await nextEpoch(t, key)
      const handle = encodeHandle({
        deployment: this.#deployment,
        key,
        node: null,
        nodeName: null,
        epoch,
        region,
        edgeHost: null,
        controlHost: null,
        ports,
      })
      await t.execute(sql`
        INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb, handle)
        VALUES (${key}, NULL, ${epoch}, 'released', ${region}, ${request.memoryMb}, ${request.cpuMillis},
                ${request.diskGb}, ${handle})`)
      await event(t, 'placement.adopted', {
        workload: key,
        epoch,
        data: { region },
      })
      return handle
    })
  }

  /**
   * The placement is made: `placed`, with the ports the node chose and the handle that names them.
   * `sent` is Blockly's digest of the spec just sent, where the caller keeps one. A spec the node
   * deferred while its copy is suspended is kept as sent, since the copy boots with it next, as a
   * Fly machine keeps the spec it slept on; the node's own digest is then unknown until that boot
   * (`booted`), so an apply meanwhile replaces whatever it runs.
   */
  async settle(
    key: string,
    node: NodeRow,
    epoch: number,
    view: WorkloadView,
    sent: { digest: string; deferred: boolean } | null = null,
  ): Promise<RuntimeHandle> {
    const ports = Object.fromEntries(view.ports.map((p) => [p.name, p.hostPort]))
    return this.#db.transaction(async (t) => {
      const row = await placementOf(t, key, true)
      if (row === null || Number(row.epoch) !== epoch || row.node_id !== node.id || row.state === 'released')
        throw new StaleHandle('The placement changed while it was being made')
      const handle = this.handleOf(
        {
          ...row,
          state: 'placed',
          ports,
          node_name: node.name,
          edge_host: node.edge_host,
          control_host: node.control_host,
        },
        view.agent ?? null,
      )
      await t.execute(sql`
        UPDATE fleet_placements SET state = 'placed', ports = ${jsonb(ports)},
          spec_digest = ${sent?.deferred ? null : view.specDigest}, sent_digest = ${sent?.digest ?? null},
          handle = ${handle}, completing = NULL, restore_from = NULL, updated_at = now()
        WHERE workload = ${key}`)
      if (row.state === 'placing')
        await event(t, 'placement.placed', {
          node: node.id,
          workload: key,
          epoch,
          data: { ports },
        })
      return handle
    })
  }

  /** The copy booted on the spec the node's digest names, which a deferred one may have become. */
  async booted(key: string, epoch: number, specDigest: string): Promise<void> {
    await this.#db.execute(sql`
      UPDATE fleet_placements SET spec_digest = ${specDigest}
      WHERE workload = ${key} AND epoch = ${epoch} AND spec_digest IS DISTINCT FROM ${specDigest}`)
  }

  /** A first placement its node refused for room: undone, as one never made is (`placement.abandoned`). */
  async undoRefused(key: string, epoch: number, nodeId: string, code: string): Promise<void> {
    await this.#db.transaction(async (t) => {
      const gone = await exec(
        t,
        sql`DELETE FROM fleet_placements WHERE workload = ${key} AND epoch = ${epoch} AND state = 'placing'`,
      )
      if (gone > 0) {
        await endHistory(t, key, epoch, `refused: ${code}`)
        await event(t, 'placement.abandoned', {
          node: nodeId,
          workload: key,
          epoch,
          data: { code },
        })
      }
    })
  }

  /** The placement rests: no node, its epoch ended, under the handle `released` names it by. */
  async release(key: string, epoch: number, nodeId: string | null, released: string): Promise<void> {
    await this.#db.transaction(async (t) => {
      await t.execute(sql`
        UPDATE fleet_placements SET state = 'released', node_id = NULL, handle = ${released}, completing = NULL,
          restore_from = NULL, sent_digest = NULL, updated_at = now() WHERE workload = ${key} AND epoch = ${epoch}`)
      await endHistory(t, key, epoch, 'released')
      await event(t, 'placement.released', {
        node: nodeId,
        workload: key,
        epoch,
      })
    })
  }

  /**
   * An operator's request that a server leave its node, for `to` if given. It takes effect at the
   * server's next relocation, which the caller asks the application for: the application stops,
   * snapshots and restarts it around the move as it does for every move.
   */
  async requestMove(sandboxId: string, to: string | null, by: string): Promise<void> {
    const moved = await exec(
      this.#db,
      sql`UPDATE fleet_placements SET move_requested_at = now(), move_to = ${to}, updated_at = now()
          WHERE workload = ${sandboxId} AND state = 'placed'`,
    )
    if (moved === 0) throw new Error('That server has no placed copy to move')
    await event(this.#db, 'placement.move_requested', {
      workload: sandboxId,
      data: { to, by },
    })
  }

  /** An operator's request to move the server, dropped: it can't be met. */
  async dropMoveRequest(row: PlacementRow): Promise<void> {
    await this.#db.execute(sql`
        UPDATE fleet_placements SET move_requested_at = NULL, move_to = NULL, updated_at = now()
        WHERE workload = ${row.workload} AND epoch = ${row.epoch}`)
  }

  /**
   * New placements a crash left `placing` after their node made the copy: the node's report of
   * that epoch is proof enough, so they are placed with the ports it reports. Never one a restore,
   * move or recovery must fill first (`completing`): its copy exists before its data. Nor one the
   * node is still making (`creating`): its image may never arrive, and the copy goes with it.
   */
  async finishReported(): Promise<number> {
    const found = await rows<ObservedRow>(
      this.#db,
      sql`${OBSERVE_SQL} WHERE p.state = 'placing' AND p.completing IS NULL AND o.epoch = p.epoch
            AND o.state <> 'creating' AND p.updated_at < now() - interval '20 seconds'`,
    )
    let finished = 0
    for (const row of found) {
      if (row.report === null) continue
      const ports = row.report.ports
      const handle = this.handleOf({ ...row, ports, handle: null })
      const placed = await exec(
        this.#db,
        sql`UPDATE fleet_placements SET state = 'placed', ports = ${jsonb(ports)}, spec_digest = ${row.report.specDigest},
              handle = ${handle}, updated_at = now()
            WHERE workload = ${row.workload} AND epoch = ${row.epoch} AND state = 'placing' AND completing IS NULL`,
      )
      if (placed === 0) continue
      await event(this.#db, 'placement.finished_from_report', {
        node: row.node_id,
        workload: row.workload,
        epoch: Number(row.epoch),
        data: { ports },
      })
      finished++
    }
    return finished
  }

  /**
   * New placements whose copy was never made: the request never reached their node. Nothing else
   * lets them go, and a placement being made holds memory on its node, so they are undone as a
   * node's refusal undoes one (`placement.abandoned`), and the next start places the server afresh.
   * Only once nothing can still be making them: untouched for longer than an operation may run (a
   * retry touches it as it claims it), and their node has beaten since then without the copy, which
   * a beat would carry if the node held it. One whose node is silent waits for it.
   */
  async abandonUnmade(): Promise<number> {
    return this.#db.transaction(async (t) => {
      const abandoned = await rows<{
        workload: string
        epoch: string
        node_id: string
      }>(
        t,
        sql`DELETE FROM fleet_placements p USING fleet_nodes n
            WHERE n.id = p.node_id AND p.state = 'placing' AND p.completing IS NULL
              AND p.updated_at < now() - make_interval(mins => ${ABANDONED_MINUTES})
              AND n.last_heartbeat_at > p.updated_at + make_interval(mins => ${ABANDONED_MINUTES})
              AND NOT EXISTS (SELECT 1 FROM fleet_observations o
                              WHERE o.node_id = p.node_id AND o.workload = p.workload AND o.epoch = p.epoch)
            RETURNING p.workload, p.epoch, p.node_id`,
      )
      for (const row of abandoned) {
        await endHistory(t, row.workload, Number(row.epoch), 'abandoned')
        await event(t, 'placement.abandoned', {
          node: row.node_id,
          workload: row.workload,
          epoch: Number(row.epoch),
          data: { code: 'never_made', minutes: ABANDONED_MINUTES },
        })
      }
      return abandoned.length
    })
  }
}
