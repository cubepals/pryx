// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Route acknowledgment adapted from Cubepals routed.ts: only counted, current edges delay a running view. */
import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import type { SandboxState } from './sandboxes.ts'
import { one } from './sql.ts'
export const EDGE_SAID_EVERY_MS = 20 * SECOND_MS
export const EDGE_SEEN_MS = 2 * EDGE_SAID_EVERY_MS + 5 * SECOND_MS
export const EDGE_BEHIND_MS = 15 * SECOND_MS
export async function shownState(db: Db, id: string, state: SandboxState): Promise<SandboxState> {
  if (state !== 'running') return state
  return (await routesPending(db, id)) ? 'starting' : state
}
export async function routesPending(db: Db, id: string): Promise<boolean> {
  const waiting = await one(
    db,
    sql`SELECT 1 FROM sandboxes b WHERE b.id=${id} AND NOT b.internal AND b.updated_at > clock_timestamp()-(${EDGE_BEHIND_MS / SECOND_MS}*interval '1 second') AND (b.spec->'agent' IS NOT NULL OR EXISTS(SELECT 1 FROM edge_addresses a WHERE a.sandbox=b.id)) AND EXISTS(SELECT 1 FROM edges e WHERE e.seen_at > clock_timestamp()-(${EDGE_SEEN_MS / SECOND_MS}*interval '1 second') AND e.first_seen_at <= b.updated_at AND (e.synced_at > e.seen_at-(${EDGE_BEHIND_MS / SECOND_MS}*interval '1 second') OR e.first_seen_at > clock_timestamp()-(${EDGE_BEHIND_MS / SECOND_MS}*interval '1 second')) AND (b.route_listed_version IS NULL OR e.held IS NULL OR e.held < b.route_listed_version))`,
  )
  return Boolean(waiting)
}
export async function sawEdge(db: Db, id: string, held: number | null) {
  await db.execute(
    sql`INSERT INTO edges(id,held,synced_at) VALUES(${id},${held},(SELECT created_at FROM edge_route_lists WHERE version=${held})) ON CONFLICT(id) DO UPDATE SET seen_at=clock_timestamp(),held=GREATEST(edges.held,excluded.held),synced_at=GREATEST(edges.synced_at,excluded.synced_at)`,
  )
}
