// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * An operator's changes to a node: drain it, hold it lost, reinstate it, retire it, clear its
 * quarantine, set its labels.
 * What the node itself reports is applied in `heartbeat.ts`, never here.
 */

import { sql } from 'drizzle-orm'

import type { Db } from '../database.ts'
import type { Thresholds } from '../health.ts'
import type { Lifecycle } from '../placement.ts'
import { event, exec, jsonb, rows } from '../sql.ts'
import { getNode, healthOf, type NodeRow, Refused } from './node.ts'
import { listeningSeconds } from './presence.ts'

// ─── lifecycle ──────────────────────────────────────────────────────────────────────────────

async function transition(
  db: Db,
  id: string,
  from: readonly Lifecycle[],
  to: Lifecycle,
  kind: string,
  data: object = {},
): Promise<NodeRow> {
  return db.transaction(async (t) => {
    const node = await getNode(t, id, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    if (!from.includes(node.lifecycle))
      throw new Refused(409, 'wrong_lifecycle', `the node is ${node.lifecycle}`)
    await t.execute(sql`UPDATE fleet_nodes SET lifecycle = ${to}, updated_at = now() WHERE id = ${id}`)
    await event(t, kind, { node: id, data: { from: node.lifecycle, ...data } })
    return { ...node, lifecycle: to }
  })
}

/** No new sandboxes; what is there stays, and moves when the application's relocation sweep moves it. */
export const drain = (db: Db, id: string, by: string) =>
  transition(db, id, ['active'], 'draining', 'node.draining', { by })
export const undrain = (db: Db, id: string, by: string) =>
  transition(db, id, ['draining'], 'active', 'node.undrained', { by })

/**
 * An operator's statement that a node is gone, and how its copies were stopped. Its placements
 * become displaced: `observe()` reports the host lost, and the application rebuilds them from
 * their newest backups once its grace has passed, where there is room. The node keeps its
 * certificate pinned: if it comes back, its first heartbeat is answered with the copies it must
 * not run, and its lease is zero.
 */
export async function confirmLost(
  db: Db,
  thresholds: Thresholds,
  id: string,
  options: { fencedBy: string; reason: string; by: string; force?: boolean },
): Promise<{ displaced: string[] }> {
  if (!options.fencedBy || !options.reason)
    throw new Refused(
      400,
      'invalid_request',
      'say how the node was stopped (fencedBy) and why it is lost (reason)',
    )
  return db.transaction(async (t) => {
    const node = await getNode(t, id, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    if (node.lifecycle !== 'active' && node.lifecycle !== 'draining')
      throw new Refused(409, 'wrong_lifecycle', `the node is ${node.lifecycle}`)
    const health = healthOf(node, thresholds, await listeningSeconds(t))
    if ((health === 'healthy' || health === 'degraded') && !options.force)
      throw new Refused(
        409,
        'node_is_beating',
        `the node beat ${Math.round(node.age ?? 0)} s ago; drain it instead`,
      )
    await t.execute(sql`
      UPDATE fleet_nodes SET lifecycle = 'lost', lost_at = now(), lost_reason = ${options.reason},
        fenced_by = ${options.fencedBy}, updated_at = now() WHERE id = ${id}`)
    const displaced = await rows<{ workload: string; epoch: string }>(
      t,
      sql`UPDATE fleet_placements SET state = 'displaced', updated_at = now()
          WHERE node_id = ${id} AND state IN ('placing', 'placed') RETURNING workload, epoch`,
    )
    for (const row of displaced)
      await event(t, 'placement.displaced', {
        node: id,
        workload: row.workload,
        epoch: Number(row.epoch),
      })
    // Its reports describe a host that is gone; a returning node reports afresh.
    await t.execute(sql`DELETE FROM fleet_observations WHERE node_id = ${id}`)
    await event(t, 'node.lost', {
      node: id,
      data: {
        fencedBy: options.fencedBy,
        reason: options.reason,
        health,
        ageSeconds: node.age,
        by: options.by,
      },
    })
    return { displaced: displaced.map((r) => r.workload) }
  })
}

/**
 * A lost node came back and its data is wanted: its sandboxes that weren't rebuilt elsewhere are
 * placed there again. One rebuilt meanwhile has a newer epoch, and its copy here is fenced. One
 * whose restore, move or rebuild here hadn't finished (`completing`) is `placing` again, so the next
 * start or retry finishes filling it rather than running it empty.
 */
export async function reinstate(db: Db, id: string, by: string): Promise<{ restored: string[] }> {
  return db.transaction(async (t) => {
    const node = await getNode(t, id, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    if (node.lifecycle !== 'lost') throw new Refused(409, 'wrong_lifecycle', `the node is ${node.lifecycle}`)
    await t.execute(sql`
      UPDATE fleet_nodes SET lifecycle = 'active', lost_at = NULL, lost_reason = NULL, fenced_by = NULL,
        updated_at = now() WHERE id = ${id}`)
    const restored = await rows<{ workload: string }>(
      t,
      sql`UPDATE fleet_placements
          SET state = (CASE WHEN completing IS NULL THEN 'placed' ELSE 'placing' END)::fleet_placement_state, updated_at = now()
          WHERE node_id = ${id} AND state = 'displaced' RETURNING workload`,
    )
    await event(t, 'node.reinstated', {
      node: id,
      data: { placements: restored.map((r) => r.workload), by },
    })
    return { restored: restored.map((r) => r.workload) }
  })
}

/**
 * A node leaves the fleet for good: its certificates stop working, so nothing it holds or sends is
 * accepted again. Only once nothing is placed on it; reinstalling a machine is a new enrollment.
 */
export async function retire(db: Db, id: string, by: string): Promise<void> {
  await db.transaction(async (t) => {
    const node = await getNode(t, id, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    if (node.lifecycle === 'retired') return
    const held = await rows<{ workload: string }>(
      t,
      sql`SELECT workload FROM fleet_placements WHERE node_id = ${id} AND state <> 'released'`,
    )
    if (held.length > 0)
      throw new Refused(409, 'not_empty', `the node still holds ${held.map((r) => r.workload).join(', ')}`)
    await t.execute(sql`
      UPDATE fleet_nodes SET lifecycle = 'retired', cert_sha256 = NULL, next_cert_sha256 = NULL,
        updated_at = now() WHERE id = ${id}`)
    await event(t, 'node.retired', {
      node: id,
      data: { from: node.lifecycle, by },
    })
  })
}

export async function clearQuarantine(db: Db, id: string, by: string): Promise<void> {
  const cleared = await exec(db, sql`UPDATE fleet_nodes SET duplicate_seen_at = NULL WHERE id = ${id}`)
  if (cleared === 0) throw new Refused(404, 'unknown_node', 'no such node')
  await event(db, 'node.quarantine_cleared', { node: id, data: { by } })
}

/**
 * An operator's labels on a node, set or removed. A node gets its first ones at enrollment, from
 * its configuration and its token; neither heartbeats nor re-enrollment change them, so this is how
 * its price (`monthly_cost_cents`, which its sandboxes' prices are a share of) is corrected.
 */
export async function setLabels(
  db: Db,
  id: string,
  change: {
    set: Record<string, string>
    remove: readonly string[]
    by: string
  },
): Promise<NodeRow> {
  if (Object.keys(change.set).length === 0 && change.remove.length === 0)
    throw new Refused(400, 'invalid_request', 'say which labels to set or remove')
  if (change.remove.some((key) => Object.hasOwn(change.set, key)))
    throw new Refused(400, 'invalid_request', 'a label is either set or removed')
  const cents = change.set.monthly_cost_cents
  if (cents !== undefined && !/^\d+$/.test(cents))
    throw new Refused(400, 'invalid_request', 'monthly_cost_cents is a whole number of US cents')
  return db.transaction(async (t) => {
    const node = await getNode(t, id, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    const labels = { ...node.labels, ...change.set }
    for (const key of change.remove) delete labels[key]
    await t.execute(
      sql`UPDATE fleet_nodes SET labels = ${jsonb(labels)}, updated_at = now() WHERE id = ${id}`,
    )
    await event(t, 'node.labels_changed', {
      node: id,
      data: { from: node.labels, to: labels, by: change.by },
    })
    return { ...node, labels }
  })
}
