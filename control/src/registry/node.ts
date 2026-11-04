// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * A node's row as the registry reads it from `fleet_nodes`, with what every part of the registry
 * shares: its options, its refusals, and the region lock placement takes.
 * It writes no row: enrollment, heartbeats, renewal and lifecycle changes each do that in their own
 * file beside this one.
 */
import { createHash } from 'node:crypto'
import type { Queryable } from '@blockly/db'
import { sql } from 'drizzle-orm'
import { deriveHealth, type Health, type Thresholds } from '../health.ts'
import type { Lifecycle } from '../placement.ts'
import { lock, one } from '../sql.ts'
import type { NodeCapacity, NodeIssue, UpgradeOffer } from '../wire.ts'

/** A request the registry turns down, with the status and code the node endpoint answers. */
export class Refused extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'Refused'
    this.status = status
    this.code = code
  }
}

export const sha256 = (data: string | Buffer) => createHash('sha256').update(data).digest('hex')

export interface RegistryOptions {
  deployment: string
  heartbeatSeconds: number
  /** The execution lease each heartbeat grants (wire.ts, `leaseSeconds`). */
  leaseSeconds: number
  /** How long a node's certificates last, and how long before their end they are renewed. */
  certDays: number
  renewDays: number
  /**
   * The blocklyd this control plane rolls out to its nodes (upgrades.ts): its own binary's
   * version and sha256. Absent with FLEET_UPGRADES=off, or without a binary to hand out.
   */
  release?: UpgradeOffer | null
}

export interface NodeRow {
  id: string
  name: string
  region_key: string
  api_address: string
  edge_host: string
  control_host: string
  lifecycle: Lifecycle
  lost_at: string | null
  lost_reason: string | null
  fenced_by: string | null
  cert_sha256: string | null
  cert_expires_at: string | null
  next_cert_sha256: string | null
  machine_id_sha256: string | null
  boot_id: string | null
  session_id: string | null
  session_started_at: string | null
  previous_session_id: string | null
  duplicate_seen_at: string | null
  heartbeat_seq: string
  last_heartbeat_at: string | null
  runtime_up: boolean
  reconciled: boolean
  health: Health
  health_since: string
  capacity: Partial<NodeCapacity>
  daemon_version: string | null
  features: string[]
  issues: NodeIssue[]
  labels: Record<string, string>
  last_probe: Record<string, unknown> | null
  upgrade_version: string | null
  upgrade_state: 'offered' | 'upgraded' | 'failed' | null
  upgrade_at: string | null
  upgrade_error: string | null
  enrolled_at: string
  /** Seconds since the last heartbeat, in database time; null if it never beat. */
  age: number | null
  /** Seconds since the session began, in database time. */
  session_age: number | null
  /** Seconds until its current certificate ends, in database time; null without one. */
  cert_seconds_left: number | null
  /** Seconds since its upgrade state last changed, in database time. */
  upgrade_age: number | null
}

export const NODE_COLUMNS = sql.raw(
  `n.*, EXTRACT(EPOCH FROM now() - n.last_heartbeat_at)::float8 AS age,
   EXTRACT(EPOCH FROM now() - n.session_started_at)::float8 AS session_age,
   EXTRACT(EPOCH FROM n.cert_expires_at - now())::float8 AS cert_seconds_left,
   EXTRACT(EPOCH FROM now() - n.upgrade_at)::float8 AS upgrade_age`,
)

export async function getNode(q: Queryable, id: string, forUpdate = false): Promise<NodeRow | null> {
  return one<NodeRow>(
    q,
    sql`SELECT ${NODE_COLUMNS} FROM fleet_nodes n WHERE n.id = ${id} ${sql.raw(forUpdate ? 'FOR UPDATE' : '')}`,
  )
}

export function healthOf(node: NodeRow, thresholds: Thresholds, listeningSeconds: number): Health {
  return deriveHealth(
    { ageSeconds: node.age, runtimeUp: node.runtime_up, reconciled: node.reconciled, listeningSeconds },
    thresholds,
  )
}

/** Placement decisions in one region run one at a time, so two can't count the same room. */
export const lockRegion = (t: Queryable, regionKey: string) => lock(t, `fleet:region:${regionKey}`)
