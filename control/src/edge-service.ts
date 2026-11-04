// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Routes, wake coalescing and activity persistence for authenticated edges; all execution stays in Sandboxes. */
import { sql } from 'drizzle-orm'
import type { z } from 'zod'
import type { PublicPort } from './api-schemas.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { addresses, DEFAULT_EDGE_PORT_RANGE } from './edge-addresses.ts'
import type { EdgeRoute, EdgeRoutes, SessionEvent, WakeResult } from './edge-schemas.ts'
import { decodeHandle } from './handle.ts'
import { fingerprint } from './operations.ts'
import type { Sandboxes, SandboxRow } from './sandboxes.ts'
import { event, jsonb, lock, one, rows } from './sql.ts'
export class EdgeService {
  readonly wakes = new Map<string, Promise<unknown>>()
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly domain: string,
    readonly portRange: readonly [number, number] = DEFAULT_EDGE_PORT_RANGE,
  ) {}
  async routes(): Promise<EdgeRoutes & { version: number; digest: string }> {
    const all = await rows<SandboxRow & { public_ports: PublicPort[] }>(
      this.db,
      sql`SELECT * FROM sandboxes WHERE deleted_at IS NULL AND NOT internal AND (expires_at IS NULL OR expires_at > now()) AND state <> 'failed' ORDER BY id`,
    )
    const routes: EdgeRoute[] = []
    for (const b of all) {
      const ref = b.handle ? decodeHandle(b.handle) : null
      for (const a of await addresses(this.db, b.id, b.public_ports, this.domain, this.portRange)) {
        const running = b.state === 'running' && ref?.node
        const endpoint = running
          ? this.boxes.engine.endpoint(b.handle as NonNullable<SandboxRow['handle']>, a.name, 'edge')
          : null
        routes.push({
          id: `${b.id}:${a.name}`,
          sandbox: b.id,
          hostname: a.hostname,
          port: a.name,
          front: a.front,
          publicPort: a.public_port,
          destination: endpoint
            ? `${endpoint.host.includes(':') ? `[${endpoint.host}]` : endpoint.host}:${endpoint.port}`
            : '0.0.0.0:0',
          epoch: ref?.epoch ?? 0,
          ...(running
            ? {}
            : {
                state:
                  b.state === 'starting' || b.state === 'restoring'
                    ? ('waking' as const)
                    : b.state === 'creating'
                      ? ('restarting' as const)
                      : ('asleep' as const),
              }),
        })
      }
    }
    const digest = fingerprint(routes)
    const version = await this.db.transaction(async (t) => {
      await lock(t, 'edge-route-list')
      let latest = await one<{ version: string; digest: string }>(
        t,
        sql`SELECT version,digest FROM edge_route_lists ORDER BY version DESC LIMIT 1`,
      )
      if (!latest || latest.digest !== digest)
        latest = await one(
          t,
          sql`INSERT INTO edge_route_lists(digest,routes) VALUES(${digest},${jsonb(routes)}) RETURNING version,digest`,
        )
      if (!latest) throw new Error('Route version was not written')
      for (const b of all.filter((b) => b.state === 'running' && routes.some((r) => r.sandbox === b.id)))
        await t.execute(
          sql`UPDATE sandboxes SET route_listed_version=COALESCE(route_listed_version,${latest.version}::bigint) WHERE id=${b.id} AND state='running' AND handle=${b.handle}`,
        )
      return Number(latest.version)
    })
    return { routes, version, digest }
  }
  async route(id: string) {
    return (await this.routes()).routes.find((r) => r.id === id)
  }
  begin(route: EdgeRoute) {
    if (!this.wakes.has(route.sandbox)) {
      const work = this.boxes.wake(route.sandbox).finally(() => this.wakes.delete(route.sandbox))
      this.wakes.set(route.sandbox, work)
      work.catch(() => {})
    }
    return this.wakes.get(route.sandbox) as Promise<unknown>
  }
  async wake(id: string): Promise<WakeResult> {
    let route = await this.route(id)
    if (!route) return { outcome: 'denied', reason: 'unknown' }
    if (route.state) {
      let timer: ReturnType<typeof setTimeout> | undefined
      try {
        await Promise.race([
          this.begin(route),
          new Promise((resolve) => {
            timer = setTimeout(resolve, 2 * SECOND_MS)
          }),
        ])
      } catch {
        return { outcome: 'denied', reason: 'failed' }
      } finally {
        clearTimeout(timer)
      }
      route = await this.route(id)
    }
    if (!route) return { outcome: 'denied', reason: 'deleted' }
    return route.state
      ? { outcome: 'starting' }
      : { outcome: 'ready', destination: route.destination, epoch: route.epoch }
  }
  async session(edge: string, body: z.infer<typeof SessionEvent>) {
    const b = await this.boxes.record(body.sandbox).catch(() => null)
    if (!b?.handle || decodeHandle(b.handle).epoch !== body.epoch) return
    const id = `${edge}:${body.id}`
    if (body.event === 'connect')
      await this.db.execute(
        sql`INSERT INTO sandbox_sessions(id,sandbox) VALUES(${id},${body.sandbox}) ON CONFLICT(id) DO UPDATE SET seen_at=now() WHERE sandbox_sessions.sandbox=excluded.sandbox`,
      )
    else await this.db.execute(sql`DELETE FROM sandbox_sessions WHERE id=${id} AND sandbox=${body.sandbox}`)
    await this.boxes.touch(body.sandbox)
  }
  async idle(sandbox: string) {
    const b = await this.boxes.record(sandbox).catch(() => null)
    if (!b) return
    await event(this.db, 'edge.idle', { workload: sandbox })
    await this.boxes.sleepIfIdle(sandbox)
  }
}
