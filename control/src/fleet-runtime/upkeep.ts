// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet upkeep: reconcile observations and copies; never choose power intent. */
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import type { Thresholds } from '../health.ts'
import type { NodeClient } from '../node-client.ts'
import { getNode, nodeSummaries, recordHealth } from '../registry.ts'
import { event, exec, jsonb, rows } from '../sql.ts'
import type { Archives } from './archives.ts'
import type { Ledger } from './ledger.ts'
import type { Rehoming } from './rehoming.ts'
import type { Transfers } from './transfers.ts'
import { address } from './workload-api.ts'

export class FleetUpkeep {
  readonly #o: { db: Db; thresholds: Thresholds; nodes: NodeClient }
  readonly #ledger: Ledger
  readonly #archives: Archives
  readonly #rehoming: Rehoming
  readonly #transfers: Transfers
  readonly #log: (message: string, fields?: object) => void

  constructor(options: { db: Db; thresholds: Thresholds; nodes: NodeClient }, ledger: Ledger,
    archives: Archives, rehoming: Rehoming, transfers: Transfers,
    log: (message: string, fields?: object) => void) {
    this.#o = options
    this.#ledger = ledger
    this.#archives = archives
    this.#rehoming = rehoming
    this.#transfers = transfers
    this.#log = log
  }

  /**
   * What keeps the fleet tidy, none of which moves or starts a server: health changes recorded,
   * placements a crash left half-made finished from their node's report, or undone when their
   * node never got them, copies nobody needs removed, snapshots uploaded off their nodes, and old
   * events dropped.
   */
  async upkeep(): Promise<void> {
    const tasks: Array<[string, () => Promise<unknown>]> = [
      ['health', () => recordHealth(this.#o.db, this.#o.thresholds)],
      ['probe', () => this.probeSilent()],
      ['finish', () => this.finishReported()],
      ['abandon', () => this.abandonUnmade()],
      ['tidy', () => this.tidy()],
      ['uploads', () => this.uploadPending()],
      ['dropped copies', () => this.dropDeletedCopies()],
      ['local copies', () => this.trimLocalCopies()],
      ['events', () => exec(this.#o.db, sql`DELETE FROM fleet_events WHERE at < now() - interval '90 days'`)],
    ]
    for (const [name, task] of tasks)
      await task().catch((error) => this.#log(`upkeep: ${name} failed`, { error: (error as Error).message }))
  }

  /** Silent nodes, probed from outside: refused (host up, blocklyd down) or timed out (host or path down). */
  async probeSilent(): Promise<void> {
    const silent = (await nodeSummaries(this.#o.db, this.#o.thresholds)).filter(
      (n) => (n.derivedHealth === 'suspect' || n.derivedHealth === 'unavailable') && n.lifecycle !== 'lost',
    )
    for (const node of silent) {
      const probe = await this.#o.nodes.probe(address(node))
      await this.#o.db.execute(sql`
        UPDATE fleet_nodes SET last_probe = ${jsonb({ ...probe, at: new Date().toISOString() })} WHERE id = ${node.id}`)
    }
  }

  /**
   * New placements a crash left `placing` after their node made the copy: the node's report of
   * that epoch is proof enough, so they are placed with the ports it reports. Never one a restore,
   * move or recovery must fill first (`completing`): its copy exists before its data.
   */
  finishReported(): Promise<number> {
    return this.#ledger.finishReported()
  }

  /** New placements whose copy was never made, undone once nothing can still be making them. */
  abandonUnmade(): Promise<number> {
    return this.#ledger.abandonUnmade()
  }

  /**
   * Copies nobody needs: those a finished move left behind, those of resting servers, and those of
   * servers destroyed while their node couldn't be reached. They are fenced, then deleted into
   * their node's trash, once the snapshots only they hold are in the store. Never an unfinished
   * move's, nor one whose host was declared lost: it may hold play the rebuild didn't, and its owner
   * decides. Nor is one with no placement and no record of its server's end: the heartbeat fences
   * it, and an operator looks.
   */
  async tidy(): Promise<number> {
    const found = await rows<{
      node_id: string
      workload: string
      epoch: string
      current_epoch: string
      end_reason: string | null
    }>(
      this.#o.db,
      sql`SELECT o.node_id, o.workload, o.epoch, p.epoch AS current_epoch, h.end_reason
          FROM fleet_observations o
          JOIN fleet_nodes n ON n.id = o.node_id
          JOIN fleet_placements p ON p.workload = o.workload
          LEFT JOIN fleet_placement_history h ON h.workload = o.workload AND h.epoch = o.epoch
          WHERE n.lifecycle IN ('active', 'draining') AND o.epoch IS NOT NULL
            AND o.node_id IS DISTINCT FROM p.node_id
            AND ((p.state = 'placed' AND p.completing IS NULL AND o.epoch < p.epoch AND h.end_reason = 'moved')
              OR (p.state = 'released' AND o.epoch <= p.epoch))
          UNION ALL
          SELECT o.node_id, o.workload, o.epoch, o.epoch + 1, 'destroyed'
          FROM fleet_observations o
          JOIN fleet_nodes n ON n.id = o.node_id
          WHERE n.lifecycle IN ('active', 'draining') AND o.epoch IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM fleet_placements p WHERE p.workload = o.workload)
            AND EXISTS (SELECT 1 FROM fleet_placement_history h
                        WHERE h.workload = o.workload AND h.end_reason = 'destroyed')`,
    )
    let tidied = 0
    for (const row of found) {
      const node = await getNode(this.#o.db, row.node_id)
      if (node === null) continue
      try {
        if (!(await this.#rehoming.deleteLeftCopy(node, row.workload, Number(row.current_epoch)))) continue
        await event(this.#o.db, 'copy.tidied', {
          node: row.node_id,
          workload: row.workload,
          epoch: Number(row.epoch),
          data: { currentEpoch: Number(row.current_epoch), why: row.end_reason ?? 'released' },
        })
        tidied++
      } catch (error) {
        this.#log('tidy failed', {
          node: row.node_id,
          workload: row.workload,
          error: (error as Error).message,
        })
      }
    }
    return tidied
  }

  /** Snapshots still only on their nodes, oldest first: a few each pass, so one slow node holds up nothing. */
  uploadPending(limit = 4): Promise<number> {
    return this.#transfers.uploadPending(limit)
  }

  /** Copies that couldn't go when their archive was deleted, and moves' copies no move will use. */
  async dropDeletedCopies(limit = 20): Promise<void> {
    await this.#transfers.dropUnusedMoveExports(limit)
    await this.#archives.dropDeletedCopies(limit)
  }

  /** Node copies of snapshots the store holds too, but the newest per server. */
  trimLocalCopies(limit = 20): Promise<number> {
    return this.#archives.trimLocalCopies(limit)
  }

}
