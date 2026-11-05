// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Direct agent HTTP/websocket relay. Verification precedes wake; exposed application forwards
 * require an assigned spare. Route and output state stay in memory; this module has no control
 * database client and makes no lifecycle call for a ready route. */
import { StatusCodes } from 'http-status-codes'
import WebSocket from 'ws'
import { SECOND_MS } from '../../control/src/durations.ts'
import { AGENT_ROUTE_PORT } from '../../control/src/edge-headers.ts'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import type { Hold } from './hold.ts'
import type { RouteTable } from './routes.ts'
import { fileToken, requestToken, sandboxToken } from './sandbox-token.ts'
import type { EdgeRoute } from './wire.ts'

type Port = { port: number; slot: number; url: string }
type SocketState = {
  upstream: WebSocket
  close: () => void
  peer?: Bun.ServerWebSocket<SocketState>
  pending: Array<string | ArrayBuffer>
  pendingBytes: number
}
// RFC 6455 section 7.4.1: a peer closes a message stream that exceeds its bound with 1009.
const MESSAGE_TOO_BIG = 1009
class ReadinessFailure extends Error {
  constructor(readonly last: string) {
    super('Agent readiness failed')
  }
}
export class SandboxProxy {
  readonly ports = new Map<string, { key: string; revision: number; ports: Port[] }>()
  readonly pending = new Map<string, Promise<Port[]>>()
  readonly server: ReturnType<typeof Bun.serve<SocketState>>
  private tlsServer?: ReturnType<typeof Bun.serve<SocketState>>
  constructor(
    readonly table: RouteTable,
    readonly hold: Hold,
    readonly holdMs: number,
    readonly activity: (route: EdgeRoute) => void = () => {},
  ) {
    this.server = this.listen()
  }
  enableTls(tls: { cert: string; key: string }) {
    if (this.tlsServer) throw new Error('Agent TLS listener already started')
    this.tlsServer = this.listen(tls)
  }
  private listen(tls?: { cert: string; key: string }) {
    return Bun.serve<SocketState>({
      tls,
      hostname: '127.0.0.1',
      port: 0,
      idleTimeout: 0,
      maxRequestBodySize: Number.MAX_SAFE_INTEGER,
      fetch: (request, server) => this.fetch(request, server),
      error: (error) => {
        console.error('Agent relay failed', error instanceof ReadinessFailure ? error.last : error.name)
        return new Response('Agent unavailable', { status: StatusCodes.BAD_GATEWAY })
      },
      websocket: {
        open: (socket) => {
          socket.data.peer = socket
          for (const message of socket.data.pending)
            if (socket.send(message) === 0) {
              socket.close()
              break
            }
          socket.data.pending = []
          socket.data.pendingBytes = 0
        },
        message: (socket, message) => {
          if (socket.data.upstream.readyState !== WebSocket.OPEN) {
            socket.close()
            return
          }
          socket.data.upstream.send(message)
          if (socket.data.upstream.bufferedAmount > 1024 * 1024) {
            socket.data.upstream.close()
            socket.close(MESSAGE_TOO_BIG)
          }
        },
        close: (socket) => {
          socket.data.upstream.close()
          socket.data.close()
        },
        drain: (socket) => {
          socket.data.upstream.resume()
          if (socket.data.upstream.bufferedAmount > 1024 * 1024)
            socket.close(MESSAGE_TOO_BIG, 'Terminal backpressure')
        },
        maxPayloadLength: 1024 * 1024,
        backpressureLimit: 1024 * 1024,
        closeOnBackpressureLimit: true,
      },
    })
  }
  destination(tls = false) {
    const server = tls ? this.tlsServer : this.server
    return server ? `127.0.0.1:${server.port}` : undefined
  }
  close() {
    this.server.stop(true)
    this.tlsServer?.stop(true)
  }

  match(host: string, path: string): EdgeRoute | undefined {
    const alias = /^\/sandboxes\/([^/]+)\/_pryx(?:\/|$)/.exec(path)
    if (alias) return this.table.held.get(`${alias[1]}:${AGENT_ROUTE_PORT}`)
    const agents = [...this.table.held.values()].filter((route) => route.agent)
    const direct = agents.find((route) => route.hostname === host && path.startsWith('/_pryx/'))
    return direct ?? agents.find((route) => this.portFromHost(host, route) !== undefined)
  }
  private portFromHost(host: string, route: EdgeRoute) {
    const suffix = route.hostname.slice(route.sandbox.length)
    if (!host.startsWith(`${route.sandbox}-`) || !host.endsWith(suffix)) return undefined
    const text = host.slice(route.sandbox.length + 1, -suffix.length)
    return /^\d+$/.test(text) ? Number(text) : undefined
  }
  private key(route: EdgeRoute) {
    return `${route.epoch}:${route.agent?.manifest.publicKey}`
  }
  private async exposed(route: EdgeRoute): Promise<Port[]> {
    const key = this.key(route),
      cached = this.ports.get(route.sandbox)
    const revision = route.agent?.manifest.portRevision ?? 0
    if (cached?.key === key && cached.revision >= revision) return cached.ports
    if (route.state || !route.agent?.routeToken)
      return this.validPorts(
        route.agent?.manifest.exposed.map((p) => ({
          ...p,
          url: `https://${route.sandbox}-${p.port}${route.hostname.slice(route.sandbox.length)}`,
        })),
        route,
      )
    if (!this.pending.has(route.sandbox)) {
      const work = (async () => {
        const response = await fetch(`http://${route.destination}/_pryx/ports`, {
          headers: { authorization: `Bearer ${route.agent?.routeToken}` },
          signal: AbortSignal.timeout(5 * SECOND_MS),
          redirect: 'manual',
        })
        if (!response.ok) return []
        const body = (await response.json()) as { ports: Port[]; revision?: number }
        const ports = this.validPorts(body.ports, route)
        const current = this.ports.get(route.sandbox)
        if (current?.key !== key || current.revision <= (body.revision ?? 0))
          this.ports.set(route.sandbox, { key, ports, revision: body.revision ?? 0 })
        return ports
      })().finally(() => this.pending.delete(route.sandbox))
      this.pending.set(route.sandbox, work)
    }
    return this.pending.get(route.sandbox) as Promise<Port[]>
  }
  private validPorts(value: unknown, route: EdgeRoute): Port[] {
    if (!Array.isArray(value)) return []
    return value.filter(
      (port): port is Port =>
        port &&
        typeof port.port === 'number' &&
        typeof port.slot === 'number' &&
        route.agent?.manifest.spares.some((spare) => spare.port === port.slot),
    )
  }

  private async ready(destination: string, token: string, signal: AbortSignal) {
    let last = 'no response'
    while (!signal.aborted) {
      try {
        const response = await fetch(`http://${destination}/_pryx/health`, {
          headers: { authorization: `Bearer ${token}` },
          signal,
          redirect: 'manual',
        })
        last = String(response.status)
        await response.body?.cancel()
        if (response.ok) return
      } catch (error) {
        last = error instanceof Error ? error.name : 'transport failure'
        if (signal.aborted) break
      }
      await Bun.sleep(SECOND_MS / 100)
    }
    throw new ReadinessFailure(last)
  }

  async fetch(
    request: Request,
    server?: ReturnType<typeof Bun.serve<SocketState>>,
  ): Promise<Response | undefined> {
    const url = new URL(request.url),
      host = (request.headers.get('host') ?? url.host).split(':')[0] as string
    const route = this.match(host, url.pathname)
    if (!route?.agent) return new Response('No such sandbox route', { status: StatusCodes.NOT_FOUND })
    const exposedPort = this.portFromHost(host, route)
    const alias = `/sandboxes/${route.sandbox}`
    const path = url.pathname.startsWith(alias) ? url.pathname.slice(alias.length) : url.pathname
    const token = requestToken(request)
    const claims = sandboxToken(token, route)
    if (exposedPort === undefined && claims?.scope !== 'sandbox' && !fileToken(claims, request, path))
      return new Response('Sandbox token refused', { status: StatusCodes.UNAUTHORIZED })
    let destination = route.destination
    if (exposedPort !== undefined) {
      let port = (await this.exposed(route)).find((port) => port.port === exposedPort)
      if (!port && !route.state) {
        this.ports.delete(route.sandbox)
        port = (await this.exposed(route)).find((port) => port.port === exposedPort)
      }
      if (!port) return new Response('Port is not exposed', { status: StatusCodes.NOT_FOUND })
      const spare = route.agent.manifest.spares.find((spare) => spare.port === port.slot)
      destination = spare?.endpoints[0] ?? ''
      if (!destination)
        return new Response('No spare destination', { status: StatusCodes.SERVICE_UNAVAILABLE })
    }
    const signal = AbortSignal.any([request.signal, AbortSignal.timeout(this.holdMs)])
    if (route.state) {
      const ready = await this.hold.ready(route, signal)
      if (exposedPort === undefined) destination = ready.destination
      // A new placement needs a new token and spare map, rather than forwarding a stale capability.
      if (ready.epoch !== route.epoch)
        return new Response('Sandbox placement changed', { status: StatusCodes.CONFLICT })
      const health = route.agent.routeToken ?? token
      if (health) await this.ready(ready.destination, health, signal)
    }
    const headers = new Headers(request.headers)
    for (const name of ['host', 'connection', 'transfer-encoding', 'content-length']) headers.delete(name)
    if (exposedPort === undefined && token) headers.set('authorization', `Bearer ${token}`)
    if (exposedPort !== undefined) {
      headers.set(nodeSchema['x-pryx-agent'].forwardHeader, String(exposedPort))
      // Each HTTP connection gets one verified target preface, removed by the agent.
      // Disable pooling here so a later request cannot pass that private header to the app.
      headers.set('connection', 'close')
      headers.set('host', request.headers.get('host') ?? url.host)
    }
    const target = `http://${destination}${path}${url.search}`
    this.activity(route)
    if (request.headers.get('upgrade')?.toLowerCase() === 'websocket') {
      if (!server) return new Response('Websocket upgrade unavailable', { status: StatusCodes.BAD_REQUEST })
      return this.websocket(
        request,
        server,
        target,
        route,
        exposedPort === undefined ? token : undefined,
        exposedPort,
      )
    }
    const cancelled = new AbortController()
    const untrack = this.table.track(route, route.destination, route.epoch, () => cancelled.abort())
    let response: Response
    try {
      response = await fetch(target, {
        method: request.method,
        headers,
        body: ['GET', 'HEAD'].includes(request.method) ? undefined : request.body,
        signal: AbortSignal.any([request.signal, cancelled.signal]),
        redirect: 'manual',
      })
    } catch (error) {
      untrack()
      throw error
    }
    if (
      exposedPort === undefined &&
      path.startsWith('/_pryx/ports') &&
      response.ok &&
      request.method !== 'GET'
    ) {
      this.ports.delete(route.sandbox)
      // Read the complete assignment after mutation. Merging a POST response can resurrect
      // another slot concurrently removed by unexpose.
      await this.exposed(route)
    }
    if (!response.body) {
      untrack()
      return response
    }
    const reader = response.body.getReader()
    return new Response(
      new ReadableStream<Uint8Array>({
        pull: async (controller) => {
          try {
            const part = await reader.read()
            if (part.done) {
              untrack()
              controller.close()
            } else controller.enqueue(part.value)
          } catch (error) {
            untrack()
            controller.error(error)
          }
        },
        cancel: async () => {
          untrack()
          await reader.cancel()
        },
      }),
      { status: response.status, statusText: response.statusText, headers: response.headers },
    )
  }

  private async websocket(
    request: Request,
    server: ReturnType<typeof Bun.serve<SocketState>>,
    target: string,
    route: EdgeRoute,
    token?: string,
    targetPort?: number,
  ) {
    const protocols = (request.headers.get('sec-websocket-protocol') ?? '')
      .split(',')
      .map((p) => p.trim())
      .filter((p) => p && !p.startsWith('pryx-token.'))
    const headers = Object.fromEntries(
      [...request.headers].filter(
        ([name]) =>
          !['host', 'connection', 'upgrade', 'content-length'].includes(name) &&
          !name.startsWith('sec-websocket-'),
      ),
    )
    if (token) headers.authorization = `Bearer ${token}`
    if (targetPort !== undefined) headers[nodeSchema['x-pryx-agent'].forwardHeader] = String(targetPort)
    const upstream = new WebSocket(target.replace(/^http:/, 'ws:'), protocols, {
      headers,
      perMessageDeflate: false,
    })
    upstream.binaryType = 'arraybuffer'
    const data: SocketState = { upstream, close: () => {}, pending: [], pendingBytes: 0 }
    let release = () => {}
    upstream.addEventListener('message', (event) => {
      const message = event.data as string | ArrayBuffer
      if (data.peer) {
        const sent = data.peer.send(message)
        if (sent === -1) upstream.pause()
        if (sent === 0) {
          upstream.close()
          data.peer.close()
        }
      } else {
        data.pendingBytes += typeof message === 'string' ? Buffer.byteLength(message) : message.byteLength
        if (data.pendingBytes > 1024 * 1024) {
          upstream.close()
          release()
          return
        }
        data.pending.push(message)
      }
    })
    upstream.addEventListener('close', () => {
      data.peer?.close()
      release()
    })
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        upstream.close()
        reject(new Error('Agent websocket timeout'))
      }, this.holdMs)
      upstream.addEventListener(
        'open',
        () => {
          clearTimeout(timer)
          resolve()
        },
        { once: true },
      )
      upstream.addEventListener(
        'error',
        () => {
          clearTimeout(timer)
          reject(new Error('Agent websocket failed'))
        },
        { once: true },
      )
    })
    release = this.table.track(route, route.destination, route.epoch, () => {
      upstream.close()
      data.peer?.close()
    })
    data.close = release
    if (upstream.readyState !== WebSocket.OPEN) {
      release()
      return new Response('Route changed', { status: StatusCodes.CONFLICT })
    }
    const upgraded = server.upgrade(request, {
      data,
      ...(upstream.protocol ? { headers: { 'sec-websocket-protocol': upstream.protocol } } : {}),
    })
    if (!upgraded) {
      upstream.close()
      release()
      return new Response('Upgrade refused', { status: StatusCodes.BAD_REQUEST })
    }
    return undefined
  }
}
