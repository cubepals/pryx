// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Whether every edge holds a running server's route, so that it may read as running: a player
 * who joins the moment it does is let in, never closed by an edge that hasn't heard yet.
 *
 * Route lists are versioned by the database's time as their read began (`inputs.ts`). An edge says which version
 * it holds every route of (`edges.synced_to`). A server's `route_listed_at` is the version that
 * first named it running, taken after the read that did, so an edge holding that version or a
 * later one holds its route. It reads as running once every counted edge does, and is told so
 * once (`routed_at`). An edge counts while it asks and keeps up:
 *
 * - one not seen for `EDGE_SEEN_MS`, or whose version was older than `EDGE_BEHIND_MS` when it last
 *   said, doesn't count,
 *   and an admin alert says so (`edgesBehind`);
 * - one first seen after a server became running doesn't hold that server back;
 * - and no server waits more than `EDGE_BEHIND_MS` from becoming running, whatever the edges say.
 *
 * With no edge counted, as where a deployment runs none, nothing waits. Not for what an edge does
 * with its routes, which is apps/edge's, or for building them (`service.ts`).
 */
import { DAY_MS, MINUTE_MS, SECOND_MS } from '@blockly/contracts/durations'
import { type Db, type Queryable, schema } from '@blockly/db'
import { and, eq, inArray, isNull, lt, type SQL, sql } from 'drizzle-orm'
import type { ServerStatus } from '../../domain/server/lifecycle.ts'
import type { MinecraftServer } from '../../domain/server/server.ts'
import type { EventBus } from '../ports/events.ts'

const { edges, minecraftServers: servers } = schema

/**
 * How often an edge's word is written while no server waits on the edges (`service.ts`): it asks
 * every second, and each word is a write. A server that waits is written for on every ask.
 */
export const EDGE_SAID_EVERY_MS = 20 * SECOND_MS

/**
 * How long an edge counts after it last said so: two of its words missed, and a margin. A word
 * that is still to come as a server turns running is the next ask's, a second away.
 */
const EDGE_SEEN_MS = 2 * EDGE_SAID_EVERY_MS + 5 * SECOND_MS

/**
 * How far an edge's version may fall behind before it stops counting; and so the longest a server
 * waits on the edges to read as running.
 */
export const EDGE_BEHIND_MS = 15 * SECOND_MS

/**
 * How long after its agent (re)starts an edge is left out of the `edge_behind` alert: a deploy
 * restarts the machine, and it takes a while to hold the newest routes again.
 */
export const EDGE_RESTART_GRACE_MS = MINUTE_MS

/** How long a row is kept for an edge gone quiet, as one replaced by a deploy. */
const EDGE_KEPT_MS = DAY_MS

/** The database's clock, `ms` before now. */
const ago = (ms: number): SQL => sql`clock_timestamp() - make_interval(secs => ${ms / SECOND_MS})`

/**
 * Whether the edge in the query keeps up: the version it held when it last said so was recent, or
 * it is new enough to be catching up. Judged against that word and not against now, which a word
 * written every `EDGE_SAID_EVERY_MS` would always find too old.
 */
const keepsUp = sql`(coalesce(${edges.syncedTo} > ${edges.seenAt} - make_interval(secs => ${EDGE_BEHIND_MS / SECOND_MS}), false)
  or (${edges.syncedTo} is null and ${edges.firstSeenAt} > ${ago(EDGE_BEHIND_MS)}))`

/** Whether the edge in the query was seen lately. */
const live = sql`${edges.seenAt} > ${ago(EDGE_SEEN_MS)}`

/** Whether the server in the query is running and held by every edge that counts for it. */
const routed = sql`${servers.status} = 'running' and (
  ${servers.routedAt} is not null
  or coalesce(${servers.readyAt}, 'epoch'::timestamptz) <= ${ago(EDGE_BEHIND_MS)}
  or not exists (
    select 1 from ${edges} where ${live} and ${keepsUp}
      and ${edges.firstSeenAt} <= coalesce(${servers.readyAt}, 'epoch'::timestamptz)
      and (${servers.routeListedAt} is null or ${edges.syncedTo} is null or ${edges.syncedTo} < ${servers.routeListedAt})
  ))`

/**
 * The status each server is shown with, by id, in one read: running only once every edge holds
 * its route; until then it is still starting, as far as anyone joining can tell.
 */
export async function shownStatuses<S extends string>(
  q: Queryable,
  list: readonly { id: string; status: S }[],
): Promise<Map<string, S | 'starting'>> {
  const running = list.filter((s) => s.status === 'running').map((s) => s.id)
  const rows =
    running.length === 0
      ? []
      : await q
          .select({ id: servers.id })
          .from(servers)
          .where(and(inArray(servers.id, running), routed))
  const held = new Set(rows.map((r) => r.id))
  return new Map(
    list.map((s) => [s.id, s.status !== 'running' || held.has(s.id) ? s.status : ('starting' as const)]),
  )
}

/** The status a server read with `shownOf` is shown with. */
export type Shown = (server: MinecraftServer) => ServerStatus

/** The statuses these servers are shown with (`shownStatuses`), in one read. */
export async function shownOf(q: Queryable, list: readonly MinecraftServer[]): Promise<Shown> {
  const shown = await shownStatuses(
    q,
    list.map((s) => ({ id: s.id, status: s.lifecycle.status })),
  )
  return (server) => shown.get(server.id) ?? server.lifecycle.status
}

/** One server's status as it is shown (`shownStatuses`). */
export async function shownStatus<S extends string>(
  q: Queryable,
  serverId: string,
  status: S,
): Promise<S | 'starting'> {
  return (await shownStatuses(q, [{ id: serverId, status }])).get(serverId) ?? status
}

/** A route list just named these servers running: the first one to do so for each says when. */
export async function markListed(q: Queryable, serverIds: readonly string[]): Promise<void> {
  if (serverIds.length === 0) return
  // Read after the list's own read, so every version from this one on names them running too.
  await q
    .update(servers)
    .set({ routeListedAt: sql`clock_timestamp()` })
    .where(
      and(inArray(servers.id, [...serverIds]), eq(servers.status, 'running'), isNull(servers.routeListedAt)),
    )
}

/**
 * Edge `edgeId` asked for its routes, holding every route of version `syncedTo`, or of none it
 * has said yet (null). Then each server every counted edge now holds, or that has waited as long
 * as any does, reads as running: its version moves, and its page is told.
 */
export async function seen(
  db: Db,
  events: EventBus,
  edgeId: string,
  syncedTo: Date | null,
  startedAt: Date | null = null,
): Promise<void> {
  await sawEdge(db, edgeId, syncedTo, startedAt)
  await forgetQuietEdges(db)
  await tellRouted(db, events)
}

/** Edge `edgeId` asked for its routes, holding every route of version `syncedTo`, or of none yet. */
export async function sawEdge(
  db: Db,
  edgeId: string,
  syncedTo: Date | null,
  startedAt: Date | null = null,
): Promise<void> {
  await db
    .insert(edges)
    .values({ edgeId, syncedTo, startedAt, seenAt: sql`clock_timestamp()` })
    .onConflictDoUpdate({
      target: edges.edgeId,
      set: {
        seenAt: sql`clock_timestamp()`,
        // A restart inside the grace of the last doesn't extend it, so an agent that crashes every few
        // seconds is reported once a grace has passed.
        ...(startedAt === null
          ? {}
          : {
              startedAt: sql`case when ${edges.startedAt} > ${ago(EDGE_RESTART_GRACE_MS)} then ${edges.startedAt} else ${startedAt} end`,
            }),
        // A version only moves forward: an answer that arrives late says nothing new.
        ...(syncedTo === null ? {} : { syncedTo: sql`greatest(${edges.syncedTo}, ${syncedTo})` }),
      },
    })
}

/** Lets go of the edges that stopped asking long ago, as ones a deploy replaced. */
export async function forgetQuietEdges(db: Db): Promise<void> {
  await db.delete(edges).where(lt(edges.seenAt, ago(EDGE_KEPT_MS)))
}

/** Each server every counted edge now holds, or that has waited as long as any does, is told so. */
async function tellRouted(db: Db, events: EventBus): Promise<void> {
  await db.transaction(async (tx) => {
    const told = await tx
      .update(servers)
      .set({ routedAt: sql`clock_timestamp()`, version: sql`${servers.version} + 1` })
      .where(and(isNull(servers.routedAt), routed))
      .returning({ id: servers.id, ownerId: servers.ownerId, version: servers.version })
    for (const server of told)
      await events.publish(tx, {
        type: 'server_changed',
        serverId: server.id,
        ownerId: server.ownerId,
        status: 'running',
        version: server.version,
      })
  })
}

/**
 * Edges still asking for routes whose version has fallen too far behind to count, bar one whose
 * agent started within `EDGE_RESTART_GRACE_MS`: it is catching up, not stuck.
 */
export async function edgesBehind(q: Queryable): Promise<string[]> {
  const rows = await q
    .select({ edgeId: edges.edgeId })
    .from(edges)
    .where(
      sql`${live} and not ${keepsUp} and (${edges.startedAt} is null or ${edges.startedAt} <= ${ago(EDGE_RESTART_GRACE_MS)})`,
    )
  return rows.map((r) => r.edgeId)
}
