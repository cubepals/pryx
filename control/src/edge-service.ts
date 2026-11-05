// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Routes, wake coalescing and activity persistence for authenticated edges; all execution stays in Sandboxes. */
import { sql } from 'drizzle-orm'
import type { z } from 'zod'
import type { Agents } from './agents.ts'
import type { PublicPort } from './api-schemas.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { addresses, DEFAULT_EDGE_PORT_RANGE } from './edge-addresses.ts'
import { AGENT_ROUTE_PORT, HTTP_FRONT_PORT } from './edge-headers.ts'
import type { EdgeRoute, EdgeRoutes, SessionEvent, WakeResult } from './edge-schemas.ts'
import { decodeHandle } from './handle.ts'
import { agentViewSchema } from './node-contract.ts'
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
    readonly agents?: Agents,
  ) {}
  async routes(includeAgents = false): Promise<EdgeRoutes & { version: number; digest: string }> {
    const all = await rows<SandboxRow & { public_ports: PublicPort[]; observed_agent: unknown }>(
      this.db,
      sql`SELECT b.*, o.report->'agent' AS observed_agent FROM sandboxes b
        LEFT JOIN fleet_placements p ON p.workload=b.id
        LEFT JOIN fleet_observations o ON o.workload=p.workload AND o.node_id=p.node_id AND o.epoch=p.epoch
        WHERE b.deleted_at IS NULL AND NOT b.internal AND (b.expires_at IS NULL OR b.expires_at > now()) AND b.state <> 'failed' ORDER BY b.id`,
    )
    const routes: EdgeRoute[] = []
    for (const b of all) {
      const ref = b.handle ? decodeHandle(b.handle) : null
      const observed = agentViewSchema.safeParse(b.observed_agent)
      if (ref?.agent && observed.success && observed.data.publicKey === ref.agent.publicKey)
        ref.agent = observed.data
      // Lifecycle activity travels on the existing heartbeat, but must not create a new
      // persisted route list every second. Zero is the protocol's unknown/default sentinel.
      if (ref?.agent) ref.agent = { ...ref.agent, activityAt: 0, activeStreams: 0 }
      if (includeAgents && ref?.agent) {
        const running = b.state === 'running' && ref.node
        routes.push({
          id: `${b.id}:${AGENT_ROUTE_PORT}`,
          sandbox: b.id,
          hostname: `${b.id}.${this.domain}`,
          port: AGENT_ROUTE_PORT,
          front: 'http',
          publicPort: HTTP_FRONT_PORT,
          epoch: ref.epoch,
          destination: ref.agent.port.endpoints[0] ?? '0.0.0.0:0',
          agent: { manifest: ref.agent },
          ...(running ? {} : { state: 'asleep' as const }),
        })
      }
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
    const digest = fingerprint(
      routes.some((r) => r.agent) ? { routes, capabilities: this.agents?.capabilityVersion() } : routes,
    )
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
      for (const b of all.filter((b) => routes.some((r) => r.sandbox === b.id)))
        await t.execute(
          sql`UPDATE sandboxes SET route_listed_version=COALESCE(route_listed_version,${latest.version}::bigint) WHERE id=${b.id} AND state=${b.state} AND handle=${b.handle}`,
        )
      return Number(latest.version)
    })
    if (this.agents) {
      const active = new Set(all.map((b) => b.id))
      for (const key of this.agents.routeTokens.keys())
        if (!active.has(key.split(':')[0] as string)) this.agents.routeTokens.delete(key)
      for (const route of routes) {
        const box = all.find((b) => b.id === route.sandbox)
        if (route.agent && box?.handle)
          route.agent.routeToken = await this.agents.routeToken(decodeHandle(box.handle))
      }
    }
    return { routes, version, digest }
  }
  async route(id: string) {
    return (await this.routes(true)).routes.find((r) => r.id === id)
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
