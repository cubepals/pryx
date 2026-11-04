// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Room to run on a node (docs/fleet.md, "Admission"): claimed in the ledger, under the region's
 * lock, before a node is asked to start a copy, so two starts can't both take the last of it; and
 * given back when the copy stops, or when the node, the final arbiter, refuses it for room. It
 * owns the claim's columns of a placement: `desired_power`, `power_changed_at`, and the memory and
 * CPU it claims. The node's answer to a power command counts as its report of the copy at once
 * (`answered`), so a copy that stopped frees its room before the node's next heartbeat.
 *
 * Choosing a node is `node-choice.ts`'s; moving a server to one with room is `rehoming.ts`'s.
 */
import type { Db, Queryable } from '@blockly/db'
import { sql } from 'drizzle-orm'
import { RuntimeFull, type RuntimeSpec } from '../../../app/ports/runtime.ts'
import { NodeRefused, NodeUnreachable } from '../node-client.ts'
import type { PlacementRequest } from '../placement.ts'
import { answered, lockRegion, type NodeRow } from '../registry.ts'
import { event, rows } from '../sql.ts'
import type { EnsureResponse, PowerResponse } from '../wire.ts'
import { type Ledger, type PlacementRow, placementOf, StaleHandle } from './ledger.ts'
import type { NodeChoice } from './node-choice.ts'
import type { WorkloadApi } from './workload-api.ts'

/** A node's refusals that mean "no room here", which the port calls RuntimeFull. */
export const NO_ROOM = new Set(['insufficient_capacity', 'no_free_ports', 'insufficient_disk'])

export class Admission {
  readonly #db: Db
  readonly #api: Pick<WorkloadApi, 'put' | 'power' | 'view'>
  readonly #choice: Pick<NodeChoice, 'refusalToStart'>
  readonly #ledger: Pick<Ledger, 'undoRefused'>
  readonly #provider: string

  constructor(options: {
    db: Db
    api: Pick<WorkloadApi, 'put' | 'power' | 'view'>
    choice: Pick<NodeChoice, 'refusalToStart'>
    ledger: Pick<Ledger, 'undoRefused'>
    provider: string
  }) {
    this.#db = options.db
    this.#api = options.api
    this.#choice = options.choice
    this.#ledger = options.ledger
    this.#provider = options.provider
  }

  /**
   * Claims room to run on the server's node, in the ledger and under its region's lock, before the
   * node is asked to start it, so two starts can't both take the last of it (docs/fleet.md,
   * "Admission"). Null once claimed; else why its node has no room for it now.
   */
  async claim(
    t: Queryable,
    row: PlacementRow,
    request: Pick<PlacementRequest, 'memoryMb' | 'cpuMillis'>,
  ): Promise<string | null> {
    const why = await this.#choice.refusalToStart(t, row, request)
    if (why !== null) return why
    await t.execute(sql`
      UPDATE fleet_placements SET desired_power = 'running', power_changed_at = now(),
        memory_mb = ${request.memoryMb}, cpu_millis = ${request.cpuMillis}, updated_at = now()
      WHERE workload = ${row.workload} AND epoch = ${row.epoch}`)
    return null
  }

  /**
   * The room a moving server's new home was chosen for, claimed in the transaction that chose it,
   * so nothing takes it while its world moves and the start that follows finds it.
   */
  async claimMoved(t: Queryable, key: string, epoch: number): Promise<void> {
    await t.execute(sql`
      UPDATE fleet_placements SET desired_power = 'running', power_changed_at = now()
      WHERE workload = ${key} AND epoch = ${epoch}`)
  }

  /** The claim given back: the copy was stopped, or its node refused to start it. */
  async unclaim(key: string, epoch: number): Promise<void> {
    await this.#db.execute(sql`
      UPDATE fleet_placements SET desired_power = 'stopped', power_changed_at = now(), updated_at = now()
      WHERE workload = ${key} AND epoch = ${epoch}`)
  }

  /** The server's compute is gone from its node, so it claims nothing, whatever its epoch. */
  async stopped(key: string): Promise<void> {
    await this.#db.execute(
      sql`UPDATE fleet_placements SET desired_power = 'stopped', power_changed_at = now() WHERE workload = ${key}`,
    )
  }

  /**
   * The spec to the node. The node's admission is the final arbiter: a first placement it refuses
   * for room is undone, and a refusal for room is RuntimeFull.
   */
  async put(
    node: NodeRow,
    key: string,
    spec: RuntimeSpec,
    epoch: number,
    created: boolean,
  ): Promise<EnsureResponse> {
    try {
      return await this.#api.put(node, key, spec, epoch)
    } catch (error) {
      if (error instanceof NodeRefused && NO_ROOM.has(error.code)) {
        // The node's admission is the final arbiter: a first placement it refuses is undone.
        if (created) await this.#ledger.undoRefused(key, epoch, node.id, error.code)
        throw new RuntimeFull(this.#provider, `${node.name}: ${error.message}`)
      }
      throw error
    }
  }

  /**
   * A power command to the node. A start's claim on the node's memory is written before it is sent
   * (`claim`); a stop or a kill gives the claim back once the node has done it, and the node's
   * answer saying the copy no longer runs frees its room at once.
   *
   * A kill is what is asked when a stop failed for good, so that nothing runs that nobody asked
   * for. When the node can't be reached for it, the claim is given back all the same and the kill
   * ends without its answer: the server rests stopped on what it ran, as on Fly. A node out of
   * reach restarts nothing once its lease runs out, and a copy still running when it is back is
   * stopped then, as stray compute behind a stopped server is.
   */
  async power(
    node: NodeRow,
    key: string,
    epoch: number,
    verb: 'start' | 'stop' | 'kill',
  ): Promise<PowerResponse | null> {
    let response: PowerResponse
    try {
      response = await this.#api.power(node, key, epoch, verb)
    } catch (error) {
      if (!(verb === 'kill' && error instanceof NodeUnreachable)) throw error
      await this.unclaim(key, epoch)
      await event(this.#db, 'power.unanswered', {
        node: node.id,
        workload: key,
        epoch,
        data: { verb, error: error.message },
      })
      return null
    }
    if (verb !== 'start') await this.unclaim(key, epoch)
    await answered(this.#db, node.id, response.workload)
    return response
  }

  /**
   * Asks a node the ledger counts full how its copies are now, before a start moves a server off it
   * for room: a copy that stopped since the node last reported it frees its room here. True when
   * any report changed. A copy the node can't say anything about keeps its last report.
   */
  async lookAgain(node: NodeRow): Promise<boolean> {
    const held = await rows<{ workload: string }>(
      this.#db,
      sql`SELECT workload FROM fleet_observations
          WHERE node_id = ${node.id} AND state IN ('creating', 'running', 'restarting', 'stopping', 'suspended')`,
    )
    let changed = false
    for (const { workload } of held) {
      try {
        if (await answered(this.#db, node.id, await this.#api.view(node, workload))) changed = true
      } catch (error) {
        if (!(error instanceof NodeRefused || error instanceof NodeUnreachable)) throw error
      }
    }
    return changed
  }

  /** Claims room to run on `node`, then starts the copy there: RuntimeFull when it has none now. */
  async startOn(node: NodeRow, key: string, epoch: number, region: string): Promise<void> {
    const why = await this.#db.transaction(async (t) => {
      await lockRegion(t, region)
      const row = await placementOf(t, key, true)
      if (row === null || Number(row.epoch) !== epoch || row.node_id !== node.id)
        throw new StaleHandle('The placement changed before it could start')
      return this.claim(t, row, { memoryMb: row.memory_mb, cpuMillis: row.cpu_millis })
    })
    if (why !== null) throw new RuntimeFull(this.#provider, `${node.name}: ${why}`)
    try {
      await this.power(node, key, epoch, 'start')
    } catch (error) {
      if (!(error instanceof NodeRefused && NO_ROOM.has(error.code))) throw error
      await this.unclaim(key, epoch)
      throw new RuntimeFull(this.#provider, `${node.name}: ${error.message}`)
    }
  }
}
