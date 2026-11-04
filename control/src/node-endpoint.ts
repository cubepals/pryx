// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Where nodes reach the control plane (pryxd's docs/protocol.md, "The control plane's
 * endpoint"): HTTPS with the fleet CA's certificate, on the deployment's private network.
 *
 * - `POST /fleet/v1/enroll`: a one-time token and a CSR, with no client certificate yet.
 * - `POST /fleet/v1/nodes/:id/heartbeat` and `…/renew`: the node's current client certificate,
 *   verified against the fleet CA, for exactly that node's name, and pinned in the registry.
 * - `GET /fleet/v1/ca.pem`, `…/join.sh`, `…/pryxd` and `…/blocklyd.sha256`: what a host
 *   fetches to join, before it has anything (join-routes.ts). An enrolled node upgrades from the
 *   same `…/pryxd` when a heartbeat's answer offers it (registry/upgrades.ts), or when its
 *   `pryxd upgrade` asks for `…/blocklyd.json`.
 */

import { createHash, randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { createServer, type Server } from 'node:https'
import { hostname } from 'node:os'
import type { TLSSocket } from 'node:tls'
import { type HttpBindings, serve } from '@hono/node-server'
import { Hono } from 'hono'
import { bodyLimit } from 'hono/body-limit'
import { endpointName, type FleetCa, isClientCertFor, nodeName } from './ca.ts'
import type { Db } from './database.ts'
import { DAY_MS } from './durations.ts'
import { joinRoutes, pryxdRelease } from './join-routes.ts'
import { endpointSeen, enroll, heartbeat, Refused, type RegistryOptions, renew } from './registry.ts'
import type { EnrollRequest, HeartbeatRequest, RenewRequest, UpgradeOffer } from './wire.ts'

export interface NodeEndpointOptions {
  db: Db
  ca: FleetCa
  registry: RegistryOptions
  listen: { host: string; port: number }
  /** The names and addresses nodes dial the endpoint by, for its certificate. */
  names: { dns: readonly string[]; ips: readonly string[] }
  /** The static pryxd hosts download as they join (PRYX_PRYXD_BIN). */
  pryxdBin: string
  /** Offer that pryxd to nodes running an older one (PRYX_UPGRADES). */
  upgrades: boolean
}

/** How long the endpoint's own certificate lasts; a fresh one replaces it every day. */
const ENDPOINT_CERT_DAYS = 7
const PRESENCE_MS = 5_000
const MAX_BODY = 4 * 1024 * 1024

export interface EndpointStats {
  heartbeats: number
  refused: number
  /** The slowest heartbeat since the process started, and the running mean, in milliseconds. */
  maxMs: number
  meanMs: number
}

type Env = { Bindings: HttpBindings }

/** The verified client certificate on this request's connection, if it is the node's own. */
function peerOf(incoming: HttpBindings['incoming'], nodeId: string, deployment: string): string {
  const socket = incoming.socket as TLSSocket
  if (!socket.authorized)
    throw new Refused(401, 'certificate_required', 'a client certificate from the fleet CA is required')
  const peer = socket.getPeerCertificate(true)
  if (!peer?.raw) throw new Refused(401, 'certificate_required', 'a client certificate is required')
  if (!isClientCertFor(peer.raw, nodeName(nodeId, deployment)))
    throw new Refused(403, 'wrong_identity', 'the certificate is not this node’s client certificate')
  return createHash('sha256').update(peer.raw).digest('hex')
}

export async function startNodeEndpoint(options: NodeEndpointOptions): Promise<{
  stop: () => Promise<void>
  stats: () => EndpointStats
  release: UpgradeOffer | null
}> {
  const { db, ca } = options
  // `pryxd upgrade` asks for it whether or not the rollout offers it.
  const pryxd = await pryxdRelease(options.pryxdBin)
  const release = options.upgrades ? pryxd : null
  const registry = { ...options.registry, release }
  const stats = { heartbeats: 0, refused: 0, maxMs: 0, totalMs: 0 }
  const app = new Hono<Env>()
  app.use(
    '*',
    bodyLimit({
      maxSize: MAX_BODY,
      onError: (c) => c.json({ error: { code: 'too_large', message: 'the body is too large' } }, 413),
    }),
  )
  app.onError((error, c) => {
    if (error instanceof Refused) {
      stats.refused++
      return c.json({ error: { code: error.code, message: error.message } }, error.status as 400)
    }
    console.error('fleet: node endpoint failed', error)
    return c.json(
      {
        error: {
          code: 'internal',
          message: 'the control plane failed; try again',
        },
      },
      500,
    )
  })
  const body = async <T>(c: { req: { json: () => Promise<unknown> } }): Promise<T> => {
    try {
      return (await c.req.json()) as T
    } catch {
      throw new Refused(400, 'invalid_json', 'the body is not JSON')
    }
  }

  app.route('/', joinRoutes({ caPem: ca.pem, pryxdBin: options.pryxdBin, release: pryxd }))

  app.post('/fleet/v1/enroll', async (c) =>
    c.json(await enroll(db, ca, registry, await body<EnrollRequest>(c))),
  )

  app.post('/fleet/v1/nodes/:id/heartbeat', async (c) => {
    const started = performance.now()
    const id = c.req.param('id')
    const peer = peerOf(c.env.incoming, id, registry.deployment)
    const answer = await heartbeat(db, registry, id, peer, await body<HeartbeatRequest>(c))
    const ms = performance.now() - started
    stats.heartbeats++
    stats.totalMs += ms
    stats.maxMs = Math.max(stats.maxMs, ms)
    return c.json(answer)
  })

  app.post('/fleet/v1/nodes/:id/renew', async (c) => {
    const id = c.req.param('id')
    const peer = peerOf(c.env.incoming, id, registry.deployment)
    return c.json(await renew(db, ca, registry, id, peer, await body<RenewRequest>(c)))
  })

  const identity = () =>
    ca.identity(endpointName(registry.deployment), 'server', ENDPOINT_CERT_DAYS, {
      dns: options.names.dns,
      ips: options.names.ips,
    })
  const first = await identity()
  const server = serve({
    fetch: app.fetch,
    hostname: options.listen.host,
    port: options.listen.port,
    createServer,
    serverOptions: {
      key: first.keyPem,
      cert: first.certPem,
      ca: [ca.pem],
      // Enrollment has no client certificate yet; heartbeats check theirs themselves.
      requestCert: true,
      rejectUnauthorized: false,
      minVersion: 'TLSv1.3',
    },
  }) as unknown as Server
  // serve() returns before the port is bound. Whoever is told the endpoint listens may dial it at
  // once (a node joining right after a restart), and a port that can't be bound fails the start.
  if (!server.listening) await once(server, 'listening')
  const renewal = setInterval(() => {
    identity()
      .then((next) =>
        server.setSecureContext({
          key: next.keyPem,
          cert: next.certPem,
          ca: [ca.pem],
        }),
      )
      .catch((error) => console.error('fleet: the node endpoint kept its certificate', error))
  }, DAY_MS)
  renewal.unref()

  // While this process serves nodes, it says so, so a node's silence is held against it only then.
  const processId = `${hostname()}:${process.pid}:${randomBytes(4).toString('hex')}`
  const startedAt = new Date()
  const seen = () =>
    endpointSeen(db, processId, startedAt).catch((error) =>
      console.error('fleet: presence not recorded', error),
    )
  await seen()
  const presence = setInterval(seen, PRESENCE_MS)
  presence.unref()

  return {
    release,
    stats: () => ({
      heartbeats: stats.heartbeats,
      refused: stats.refused,
      maxMs: Math.round(stats.maxMs * 10) / 10,
      meanMs: stats.heartbeats === 0 ? 0 : Math.round((stats.totalMs / stats.heartbeats) * 10) / 10,
    }),
    stop: async () => {
      clearInterval(renewal)
      clearInterval(presence)
      await new Promise<void>((resolve) => server.close(() => resolve()))
    },
  }
}
