// SPDX-License-Identifier: FSL-1.1-ALv2
/** Current authenticated edge advertisements choose SSH transport. Legacy edges retain raw
 * behavior; stale edges cannot keep a missing raw front selected. No connection is attempted. */
import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { HTTP_ONLY_FEATURE, SSH_WEBSOCKET_FEATURE } from './edge-headers.ts'
import { EDGE_SEEN_MS } from './edge-routed.ts'
import { one } from './sql.ts'

export async function sshTransport(db: Db, agentSupportsSocket: boolean) {
  if (!agentSupportsSocket) return undefined
  const value = await one<{ websocket: boolean; raw: boolean }>(
    db,
    sql`SELECT COALESCE(bool_or(features ? ${SSH_WEBSOCKET_FEATURE}),false) AS websocket, COALESCE(bool_or(NOT(features ? ${HTTP_ONLY_FEATURE})),false) AS raw FROM edges WHERE seen_at>clock_timestamp()-(${EDGE_SEEN_MS / SECOND_MS}*interval '1 second')`,
  )
  return value?.websocket ? (value.raw ? ('tcp' as const) : ('websocket' as const)) : undefined
}
