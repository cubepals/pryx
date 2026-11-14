// SPDX-License-Identifier: FSL-1.1-ALv2
/** SSH registration proves the per-call agent lease before allocating an edge address. Only
 * public metadata is durable; the client private key never traverses this control service. */

import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import { z } from 'zod'
import type { Agents } from './agents.ts'
import type { PublicPort } from './api-schemas.ts'
import type { Db, Queryable } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { addresses, DEFAULT_EDGE_PORT_RANGE } from './edge-addresses.ts'
import { routesPending } from './edge-routed.ts'
import type { EdgeRoute } from './edge-schemas.ts'
import { sshTransport } from './edge-transports.ts'
import { decodeHandle } from './handle.ts'
import { agentSchema } from './node-contract.ts'
import { SandboxError, type Sandboxes, type SandboxRow } from './sandboxes.ts'
import { one, rows } from './sql.ts'
export const sshRequest = z.object({ id: z.uuid() }).strict()
export const sshAddress = z.object({
  id: z.uuid(),
  host: z.string(),
  port: z.number().int(),
  user: z.string(),
  hostKey: z.string(),
  expiresAt: z.number().int(),
  transport: z.enum(['tcp', 'websocket']).optional(),
  webSocketUrl: z.url().optional(),
})
type View = import('../../sdk/typescript/src/generated/agent.ts').components['schemas']['SshView']
export class NativeSsh {
  constructor(
    private db: Db,
    private boxes: Sandboxes,
    private agents: Agents,
    private domain: string,
    private range: readonly [number, number] = DEFAULT_EDGE_PORT_RANGE,
  ) {}
  async register(id: string, project: string, input: unknown, requestUrl: string) {
    const body = sshRequest.parse(input),
      box = await this.boxes.record(id, project)
    if (!box.handle || box.state !== 'running')
      throw new SandboxError('not_running', 'SSH registration needs a running sandbox')
    const ref = decodeHandle(box.handle),
      credential = await this.agents.token(id, project, requestUrl)
    const response = await fetch(`${credential.url}/_pryx/ssh/${body.id}`, {
      headers: { authorization: `Bearer ${credential.token}` },
      redirect: 'manual',
      signal: AbortSignal.timeout(5 * SECOND_MS),
    })
    if (!response.ok) {
      await response.body?.cancel()
      throw new SandboxError('unsupported', 'Agent did not confirm this SSH lease')
    }
    let view = agentSchema('SshView').parse(await response.json()) as View
    if (
      view.id !== body.id ||
      !view.running ||
      !ref.agent?.spares.some((s) => s.port === view.slot) ||
      !/^ssh-ed25519 [A-Za-z0-9+/=]+$/.test(view.hostKey) ||
      !/^[A-Za-z0-9_-]+$/.test(view.user)
    )
      throw new SandboxError('invalid_request', 'Invalid SSH lease binding')
    const expiresAt = Math.min(
      view.expiresAt,
      Math.floor(Date.now() / SECOND_MS + (box.spec.idle ?? Number.POSITIVE_INFINITY)),
    )
    if (expiresAt <= Math.floor(Date.now() / SECOND_MS)) throw new SandboxError('gone', 'SSH lease expired')
    if (expiresAt < view.expiresAt) {
      const shortened = await fetch(`${credential.url}/_pryx/ssh/${body.id}/expiry`, {
        method: 'POST',
        headers: { authorization: `Bearer ${credential.token}`, 'content-type': 'application/json' },
        body: JSON.stringify({ expiresAt }),
        redirect: 'manual',
        signal: AbortSignal.timeout(5 * SECOND_MS),
      })
      if (!shortened.ok) {
        await shortened.body?.cancel()
        throw new SandboxError('operation_unknown', 'SSH expiry was not confirmed')
      }
      view = agentSchema('SshView').parse(await shortened.json()) as View
    }
    if ((await this.boxes.record(id, project)).handle !== box.handle)
      throw new SandboxError('gone', 'Sandbox placement changed during SSH registration')
    const address = await this.db.transaction(async (t) => {
      await t.execute(
        sql`INSERT INTO sandbox_ssh(id,sandbox,project,epoch,slot,target_port,username,host_key,expires_at) VALUES(${body.id},${id},${project},${ref.epoch},${view.slot},${view.port},${view.user},${view.hostKey},${new Date(expiresAt * SECOND_MS)}) ON CONFLICT(id) DO NOTHING`,
      )
      const lease = await one<{ sandbox: string; revoked_at: string | null }>(
        t,
        sql`SELECT sandbox,revoked_at FROM sandbox_ssh WHERE id=${body.id}`,
      )
      if (lease?.sandbox !== id || lease.revoked_at) throw new SandboxError('gone', 'SSH lease was revoked')
      const port: PublicPort = {
        name: `ssh-${body.id}`,
        port: view.slot,
        protocol: 'tcp',
        audience: 'public',
        front: 'port',
      }
      const assigned = await addresses(t, id, [port], this.domain, this.range)
      const address = assigned.find((a) => a.name === port.name)
      if (!address) throw new SandboxError('no_room', 'SSH address was not allocated')
      await t.execute(
        sql`UPDATE sandboxes SET route_listed_version=NULL,updated_at=clock_timestamp() WHERE id=${id} AND handle=${box.handle}`,
      )
      return address
    })
    // A fresh port listener exists only after an edge applies and acknowledges this route list.
    while (await routesPending(this.db, id)) await pause(SECOND_MS / 20)
    if ((await this.boxes.record(id, project)).handle !== box.handle)
      throw new SandboxError('gone', 'Sandbox placement changed during SSH registration')
    const incoming = new URL(requestUrl).hostname.replace(/^\[|\]$/g, '')
    const transport = await sshTransport(this.db, view.webSocket === true)
    const socket = new URL(`${credential.url.replace(/\/$/, '')}/_pryx/ssh/${body.id}/connect`)
    socket.protocol = socket.protocol === 'https:' ? 'wss:' : 'ws:'
    return {
      id: body.id,
      host: ['127.0.0.1', 'localhost', '::1'].includes(incoming) ? incoming : address.hostname,
      port: address.public_port,
      user: view.user,
      hostKey: view.hostKey,
      expiresAt,
      ...(transport ? { transport, webSocketUrl: socket.toString() } : {}),
    }
  }
  async revoke(id: string, project: string, requestUrl: string) {
    await this.boxes.record(id, project)
    await this.db.transaction(async (t) => {
      await t.execute(
        sql`UPDATE sandbox_ssh SET revoked_at=now() WHERE sandbox=${id} AND project=${project} AND revoked_at IS NULL`,
      )
      await t.execute(
        sql`UPDATE edge_addresses SET retired_at=now() WHERE sandbox=${id} AND name IN (SELECT 'ssh-'||s.id::text FROM sandbox_ssh s WHERE s.sandbox=${id}) AND retired_at IS NULL`,
      )
    })
    // Retiring the edge route closes active sessions even when the sandbox is stopped.
    const box = await this.boxes.record(id, project)
    if (box.state === 'running') {
      const credential = await this.agents.token(id, project, requestUrl)
      const response = await fetch(`${credential.url}/_pryx/ssh`, {
        method: 'DELETE',
        headers: { authorization: `Bearer ${credential.token}` },
        redirect: 'manual',
        signal: AbortSignal.timeout(5 * SECOND_MS),
      })
      await response.body?.cancel()
      if (!response.ok)
        throw new SandboxError(
          'operation_unknown',
          'SSH routes were revoked but agent cleanup was not confirmed',
        )
    }
    return { revoked: true }
  }
}
/**
 * A sandbox's SSH lease routes and the names of every unretired address its leases hold. One
 * statement also retires the addresses of leases that expired or belong to an older epoch, so
 * their public ports reach the allocator's quarantine cleanup like revoked ones.
 */
export async function sshLeases(
  db: Queryable,
  box: SandboxRow,
): Promise<{ routes: EdgeRoute[]; addresses: Set<string> }> {
  const ref = box.handle ? decodeHandle(box.handle) : null,
    epoch = ref?.epoch ?? null
  const leases = await rows<{
    id: string
    slot: number
    target_port: number
    hostname: string
    public_port: number
    live: boolean
  }>(
    db,
    sql`WITH lapsed AS (UPDATE edge_addresses a SET retired_at=now() FROM sandbox_ssh s WHERE s.sandbox=${box.id} AND a.sandbox=s.sandbox AND a.name='ssh-'||s.id::text AND a.retired_at IS NULL AND s.revoked_at IS NULL AND (s.expires_at<=now() OR s.epoch<${epoch}::bigint) RETURNING a.name)
      SELECT s.id,s.slot,s.target_port,a.hostname,a.public_port,COALESCE(s.epoch=${epoch}::bigint AND s.revoked_at IS NULL AND s.expires_at>now(),false) AS live FROM sandbox_ssh s JOIN edge_addresses a ON a.sandbox=s.sandbox AND a.name='ssh-'||s.id::text WHERE s.sandbox=${box.id} AND a.retired_at IS NULL`,
  )
  const addresses = new Set(leases.map((lease) => `ssh-${lease.id}`))
  if (!ref) return { routes: [], addresses }
  const routes = leases.flatMap((lease) => {
    const endpoint = lease.live
      ? ref.agent?.spares.find((s) => s.port === lease.slot)?.endpoints[0]
      : undefined
    if (!endpoint) return []
    return [
      {
        id: `${box.id}:ssh-${lease.id}`,
        sandbox: box.id,
        hostname: lease.hostname,
        port: `ssh-${lease.id}`,
        front: 'port' as const,
        publicPort: lease.public_port,
        destination: endpoint,
        epoch: ref.epoch,
        ssh: { targetPort: lease.target_port, session: lease.id },
        ...(box.state === 'running' ? {} : { state: 'asleep' as const }),
      },
    ]
  })
  return { routes, addresses }
}
export async function sshRoutes(db: Queryable, box: SandboxRow): Promise<EdgeRoute[]> {
  return (await sshLeases(db, box)).routes
}
