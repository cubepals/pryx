// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Whether the node endpoint itself is up: each endpoint process records that it serves, and the
 * registry reads how long nodes have had somewhere to send heartbeats.
 * It judges no node's health; `node.ts` does, from what this file reports.
 */
import type { Db, Queryable } from '@blockly/db'
import { sql } from 'drizzle-orm'
import { one } from '../sql.ts'

// ─── the node endpoint's own presence ───────────────────────────────────────────────────────

/** How long an endpoint process may go unseen before it is taken for gone. */
const ENDPOINT_SEEN_SECONDS = 15

/** An endpoint process says it is serving: on start, and every few seconds after. */
export async function endpointSeen(db: Db, processId: string, startedAt: Date): Promise<void> {
  await db.execute(sql`
    INSERT INTO fleet_endpoints (process_id, started_at, seen_at) VALUES (${processId}, ${startedAt}, now())
    ON CONFLICT (process_id) DO UPDATE SET seen_at = now()`)
  await db.execute(sql`DELETE FROM fleet_endpoints WHERE seen_at < now() - interval '1 day'`)
}

/**
 * How long nodes have had somewhere to send heartbeats without a gap: since the oldest endpoint
 * still serving started. Zero while none serves, so no node's silence counts against it then.
 */
export async function listeningSeconds(q: Queryable): Promise<number> {
  const row = await one<{ seconds: number | null }>(
    q,
    sql`SELECT EXTRACT(EPOCH FROM now() - MIN(started_at))::float8 AS seconds FROM fleet_endpoints
        WHERE seen_at > now() - make_interval(secs => ${ENDPOINT_SEEN_SECONDS})`,
  )
  return row?.seconds ?? 0
}
