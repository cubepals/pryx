// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** TCP front and relay with bounded greetings/holds, backpressure, session reporting and stale-epoch closes. */
import { connect, createServer, type Socket } from 'node:net'
import { setTimeout as pause } from 'node:timers/promises'
import { SECOND_MS } from '../../control/src/durations.ts'
import { httpHost, tlsSni } from './greeters.ts'
import type { Hold } from './hold.ts'
import type { Gate } from './limits.ts'
import type { Metrics } from './metrics.ts'
import type { RouteTable } from './routes.ts'
import type { SandboxProxy } from './sandbox-proxy.ts'
import type { EdgeRoute } from './wire.ts'
export interface FrontOptions {
  control?: { destination: string; paths: RegExp; hostname?: string }
  agents?: SandboxProxy
  host: string
  port: number
  routePort: number
  kind: EdgeRoute['front']
  table: RouteTable
  gate: Gate
  hold: Hold
  holdMs: number
  greetingMs: number
  bufferBytes: number
  metrics: Metrics
  session: (
    id: string,
    route: EdgeRoute,
    event: 'connect' | 'disconnect',
    bytes?: { bytesUp: number; bytesDown: number },
  ) => void
}
function dial(destination: string, signal: AbortSignal): Promise<Socket> {
  const at = destination.lastIndexOf(':'),
    host = destination.slice(0, at).replace(/^\[|\]$/g, ''),
    port = Number(destination.slice(at + 1))
  return new Promise((resolve, reject) => {
    const socket = connect({ host, port })
    const abort = () => socket.destroy(new Error('Connect cancelled'))
    if (signal.aborted) abort()
    else signal.addEventListener('abort', abort, { once: true })
    socket.once('error', () => signal.removeEventListener('abort', abort))
    socket.once('error', reject)
    socket.once('connect', () => {
      signal.removeEventListener('abort', abort)
      socket.off('error', reject)
      socket.on('error', () => {})
      resolve(socket)
    })
  })
}
export async function front(options: FrontOptions) {
  const clients = new Set<Socket>()
  const server = createServer((client) => {
    clients.add(client)
    client.once('close', () => clients.delete(client))
    void arrival(client, options)
  })
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(options.port, options.host, () => {
      server.off('error', reject)
      resolve()
    })
  })
  return Object.assign(server, {
    closeClients: () => {
      for (const c of clients) c.destroy()
    },
  })
}
async function arrival(client: Socket, o: FrontOptions) {
  const releaseGreeting = o.gate.greet(client.remoteAddress ?? '')
  if (!releaseGreeting) {
    client.destroy()
    return
  }
  let releaseOpen: (() => void) | undefined,
    untrack: (() => void) | undefined,
    upstream: Socket | undefined,
    route: EdgeRoute | undefined,
    sessionId: string | undefined
  let bytes = Buffer.alloc(0),
    handed = false,
    closed = false
  const cancelled = new AbortController()
  const finish = () => {
    if (closed) return
    closed = true
    clearTimeout(deadline)
    cancelled.abort()
    releaseGreeting()
    releaseOpen?.()
    untrack?.()
    upstream?.destroy()
    if (route && sessionId)
      o.session(sessionId, route, 'disconnect', { bytesUp: client.bytesRead, bytesDown: client.bytesWritten })
  }
  client.once('close', finish)
  client.on('error', () => client.destroy())
  const deadline = setTimeout(() => client.destroy(), o.holdMs + o.greetingMs)
  const collect = (chunk: Buffer) => {
    if (handed) return
    const room = Math.max(0, o.bufferBytes - bytes.length)
    bytes = Buffer.concat([bytes, chunk.subarray(0, room)])
    if (chunk.length > room) {
      try {
        const complete = o.kind === 'http' ? httpHost(bytes) : o.kind === 'tls' ? tlsSni(bytes) : true
        if (!complete) {
          client.destroy()
          return
        }
        client.pause()
        client.unshift(chunk.subarray(room))
      } catch {
        client.destroy()
      }
    }
  }
  client.on('data', collect)
  try {
    let name = ''
    if (o.kind !== 'port') {
      const greeter = o.kind === 'http' ? httpHost : tlsSni
      name = await new Promise<string>((resolve, reject) => {
        const timer = setTimeout(() => {
          cleanup()
          reject(new Error('Greeting elapsed'))
        }, o.greetingMs)
        const inspect = () => {
          try {
            const name = greeter(bytes)
            if (name) {
              client.pause()
              cleanup()
              resolve(name)
            }
          } catch (error) {
            cleanup()
            reject(error)
          }
        }
        const abort = () => {
          cleanup()
          reject(new Error('Client left'))
        }
        const cleanup = () => {
          clearTimeout(timer)
          client.off('data', inspect)
          cancelled.signal.removeEventListener('abort', abort)
        }
        client.on('data', inspect)
        cancelled.signal.addEventListener('abort', abort, { once: true })
        inspect()
      })
    }
    client.pause()
    releaseGreeting()
    const path = bytes.toString('latin1').split(' ')[1] ?? ''
    const agentRoute =
      o.kind === 'http'
        ? o.agents?.match(name, path)
        : o.kind === 'tls' && o.agents?.destination(true)
          ? o.agents.match(name, '/_pryx/')
          : undefined
    const assigned = agentRoute ? undefined : o.table.find(name, o.kind, o.routePort)
    const fallback =
      !agentRoute &&
      o.kind === 'http' &&
      o.control?.paths.test(path) &&
      (!assigned || !o.control.hostname || o.control.hostname === name)
    route =
      agentRoute ??
      (fallback
        ? {
            id: '__control',
            sandbox: '__control',
            hostname: name,
            port: 'http',
            front: 'http',
            publicPort: o.routePort,
            destination: o.control?.destination as string,
            epoch: 0,
          }
        : assigned)
    releaseOpen = o.gate.admit(route?.sandbox ?? '__unknown__', client.remoteAddress ?? '') ?? undefined
    if (!route || !releaseOpen) throw new Error('Unknown or limited destination')
    o.metrics.holds++
    const until = AbortSignal.any([cancelled.signal, AbortSignal.timeout(o.holdMs)])
    try {
      let target = {
        destination: agentRoute ? (o.agents?.destination(o.kind === 'tls') as string) : route.destination,
        epoch: route.epoch,
      }
      if (route.state && !agentRoute && !fallback) target = await o.hold.ready(route, until)
      while (!until.aborted) {
        try {
          upstream = await dial(target.destination, until)
          break
        } catch {
          if (agentRoute) throw new Error('Agent ingress unavailable')
          await pause(SECOND_MS / 4, undefined, { signal: until })
          target = await o.hold.ready(route, until)
        }
      }
      if (!upstream || client.destroyed) throw new Error('No destination')
      client.pause()
      handed = true
      client.off('data', collect)
      route = { ...route, epoch: target.epoch, destination: target.destination }
      if (!agentRoute && !fallback)
        untrack = o.table.track(route, target.destination, target.epoch, () => {
          client.destroy()
          upstream?.destroy()
        })
      if (client.destroyed) throw new Error('Route moved')
      if (!agentRoute && !fallback) {
        sessionId = crypto.randomUUID()
        o.session(sessionId, { ...route, epoch: target.epoch }, 'connect')
      }
      upstream.once('close', () => client.destroy())
      upstream.write(bytes)
      bytes = Buffer.alloc(0)
      clearTimeout(deadline)
      client.pipe(upstream)
      upstream.pipe(client)
      client.resume()
      o.metrics.count('wakes_total', 'ready')
    } finally {
      o.metrics.holds--
    }
  } catch {
    o.metrics.count('wakes_total', cancelled.signal.aborted ? 'client_closed' : 'failed')
    client.destroy()
  } finally {
    if (!handed) clearTimeout(deadline)
  }
}
