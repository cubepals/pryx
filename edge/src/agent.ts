// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Compose route polling, TCP fronts and activity reporting. Only TLS, HTTP headers and raw bytes reach this service. */

import { StatusCodes } from 'http-status-codes'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'

import { EDGE_ID_HEADER, HTTP_FRONT_PORT, TLS_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { type EdgeConfig, edgeConfig } from './config.ts'
import { front } from './front.ts'
import { Hold } from './hold.ts'
import { Gate } from './limits.ts'
import { Metrics } from './metrics.ts'
import { RoutesPoll } from './poll.ts'
import { RouteTable } from './routes.ts'
import type { EdgeRoute, WakeResult } from './wire.ts'
export async function startEdge(config: EdgeConfig = edgeConfig()) {
  const metrics = new Metrics(),
    table = new RouteTable(() => metrics.count('stale_closes_total')),
    gate = new Gate(config.limits)
  const abort = new AbortController(),
    listeners = new Map<string, Awaited<ReturnType<typeof front>>>()
  const sessions = new Map<string, EdgeRoute>()
  const send = async (path: string, body: unknown, signal: AbortSignal = abort.signal) => {
    const r = await fetch(`${config.control.replace(/\/$/, '')}/edge/v1${path}`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${config.token}`,
        [EDGE_ID_HEADER]: config.id,
        'content-type': 'application/json',
      },
      body: JSON.stringify(body),
      signal: AbortSignal.any([signal, AbortSignal.timeout(config.requestMs)]),
    })
    if (!r.ok) throw new Error('Edge report refused')
    return r
  }
  const report = (
    id: string,
    route: EdgeRoute,
    event: 'connect' | 'disconnect',
    bytes?: { bytesUp: number; bytesDown: number },
  ) => {
    if (event === 'disconnect' || !sessions.has(id))
      void send('/visits', { reports: [{ sandbox: route.sandbox, event, ...bytes }] }).catch(() =>
        metrics.count('report_failures_total'),
      )
    if (event === 'connect') sessions.set(id, route)
    else sessions.delete(id)
    void send('/sessions', { id, sandbox: route.sandbox, epoch: route.epoch, event }).catch(() =>
      metrics.count('report_failures_total'),
    )
    if (event === 'disconnect' && ![...sessions.values()].some((r) => r.sandbox === route.sandbox))
      void send('/idle', { sandbox: route.sandbox }).catch(() => {})
  }
  const hold = new Hold(async (route, signal) => {
    const r = await send('/wake', { route }, signal)
    const result = (await r.json()) as WakeResult
    metrics.count('wake_answers_total', result.outcome)
    return result
  })
  const ensure = async (kind: EdgeRoute['front'], logicalPort: number, listenPort: number) => {
    const key = `${kind}:${logicalPort}`
    if (listeners.has(key)) return
    listeners.set(
      key,
      await front({
        host: config.host,
        port: listenPort,
        routePort: logicalPort,
        kind,
        table,
        gate,
        hold,
        holdMs: config.holdMs,
        greetingMs: config.greetingMs,
        bufferBytes: config.bufferBytes,
        metrics,
        session: report,
      }),
    )
  }
  const poll = new RoutesPoll(
    async (headers) =>
      fetch(`${config.control.replace(/\/$/, '')}/edge/v1/routes`, {
        headers: { authorization: `Bearer ${config.token}`, [EDGE_ID_HEADER]: config.id, ...headers },
        signal: AbortSignal.any([abort.signal, AbortSignal.timeout(config.requestMs)]),
      }),
    async (list) => {
      table.apply(list)
      await ensure('http', HTTP_FRONT_PORT, config.httpPort)
      await ensure('tls', TLS_FRONT_PORT, config.tlsPort)
      for (const route of list.routes)
        if (route.front === 'port' || (route.front === 'tls' && route.publicPort !== TLS_FRONT_PORT))
          await ensure(route.front, route.publicPort, route.publicPort)
      metrics.routes = table.held.size
      return true
    },
  )
  const pollOnce = async () => {
    try {
      if (await poll.sync()) await poll.sync()
      metrics.version = poll.version
    } catch {
      metrics.count('poll_failures_total')
    }
    for (const [key, count] of gate.sweep()) metrics.count('refusals_total', key.split(' ')[0], count)
  }
  await pollOnce()
  let polling = false
  const timer = setInterval(() => {
    if (polling) return
    polling = true
    void pollOnce().finally(() => {
      polling = false
    })
  }, SECOND_MS)
  const refresh = setInterval(() => {
    for (const [id, route] of sessions) report(id, route, 'connect')
  }, MINUTE_MS / 2)
  const status = Bun.serve({
    hostname: config.metricsHost,
    port: config.metricsPort,
    fetch: (r) =>
      new URL(r.url).pathname === '/metrics'
        ? new Response(metrics.render() + `pryx_edge_version_lag ${poll.current() ? 0 : 1}\n`)
        : new Response(null, { status: StatusCodes.NOT_FOUND }),
  })
  return {
    table,
    metrics,
    poll,
    listeners,
    status,
    stop: async () => {
      clearInterval(timer)
      clearInterval(refresh)
      abort.abort()
      status.stop(true)
      for (const c of table.connections) c.close()
      await Promise.all(
        [...listeners.values()].map(
          (server) =>
            new Promise<void>((resolve) => {
              server.closeClients()
              server.close(() => resolve())
            }),
        ),
      )
    },
  }
}
if (import.meta.main) {
  const edge = await startEdge()
  for (const signal of ['SIGINT', 'SIGTERM'] as const)
    process.once(signal, () => {
      void edge.stop().then(() => process.exit(0))
    })
}
