// SPDX-License-Identifier: FSL-1.1-ALv2
/** Signed capabilities and a real HTTP/websocket peer for edge transport tests. Process execution
 * itself is covered by the Rust agent tests and the container end-to-end suite. */
import { generateKeyPairSync, sign } from 'node:crypto'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import { AGENT_ROUTE_PORT } from '../../control/src/edge-headers.ts'
import node from '../../openapi/node-writes.openapi.json'
import type { components } from '../../sdk/typescript/src/generated/api.ts'
import { Hold } from '../src/hold.ts'
import { RouteTable } from '../src/routes.ts'
import { SandboxProxy } from '../src/sandbox-proxy.ts'
import { requestToken, sandboxToken } from '../src/sandbox-token.ts'
import type { EdgeRoute } from '../src/wire.ts'

export function agentContext(sandbox = 'one') {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519')
  const publicEncoded = publicKey.export({ format: 'jwk' }).x as string
  const result = {
    code: 0,
    durationMs: 1,
    timedOut: false,
    killed: false,
    stdoutTruncated: false,
    stderrTruncated: false,
  }
  const calls: string[] = [],
    frames: unknown[] = []
  const output: components['schemas']['AgentEvent'][] = [{ type: 'exit', result }]
  const logFrames: components['schemas']['AgentLogFrame'][] = []
  let file = new Uint8Array(new ArrayBuffer(0)),
    processes = 0,
    wakes = 0
  let route: EdgeRoute
  const app = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('exposed-app') })
  const agent = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request, server) {
      const path = new URL(request.url).pathname
      const claim = sandboxToken(requestToken(request), route)
      if (
        !claim ||
        (claim.scope !== 'sandbox' &&
          !(
            claim.scope === 'routes' &&
            ['/_pryx/ports', '/_pryx/health'].includes(path) &&
            request.method === 'GET'
          ))
      )
        return new Response(null, { status: StatusCodes.UNAUTHORIZED })
      calls.push(`${request.method} ${path}`)
      if (request.headers.get('upgrade') === 'websocket') {
        if (server.upgrade(request, { headers: { 'sec-websocket-protocol': 'pryx' } })) return
        return new Response(null, { status: StatusCodes.BAD_REQUEST })
      }
      if (path === '/_pryx/processes' && request.method === 'POST')
        return Response.json(
          { id: `process-${++processes}`, pid: processes, running: true, result: null },
          { status: StatusCodes.CREATED },
        )
      if (path === '/_pryx/processes') return Response.json({ processes: [] })
      if (path.endsWith('/wait')) return Response.json(result)
      if (path.endsWith('/output'))
        return new Response(output.map((event) => `${JSON.stringify(event)}\n`).join(''), {
          headers: { 'content-type': 'application/x-ndjson' },
        })
      if (path === '/_pryx/logs')
        return new Response(logFrames.map((frame) => `${JSON.stringify(frame)}\n`).join(''), {
          headers: { 'content-type': 'application/x-ndjson' },
        })
      if (path === '/_pryx/ports')
        return Response.json(
          request.method === 'POST'
            ? { port: 3000, slot: 1, url: 'https://one-3000.sandboxes.test' }
            : { ports: [{ port: 3000, slot: 1, url: 'https://one-3000.sandboxes.test' }] },
        )
      if (path === '/_pryx/files/content') {
        if (request.method === 'PUT') {
          file = new Uint8Array(await request.arrayBuffer())
          return new Response(null, { status: StatusCodes.NO_CONTENT })
        }
        return new Response(file)
      }
      if (path === '/_pryx/files/list')
        return Response.json({ files: [{ name: 'data', kind: 'file', size: file.length, modified: 0 }] })
      return Response.json({ status: 'ok' })
    },
    websocket: {
      open: (socket) => {
        socket.send(JSON.stringify({ type: 'output', stream: 'stdout', bytes: btoa('prompt\n') }))
      },
      message: (socket, message) => {
        const frame = JSON.parse(String(message))
        frames.push(frame)
        if (frame.type === 'stdin')
          socket.send(JSON.stringify({ type: 'output', stream: 'stdout', bytes: frame.bytes }))
      },
    },
  })
  const table = new RouteTable()
  route = {
    id: `${sandbox}:${AGENT_ROUTE_PORT}`,
    sandbox,
    hostname: `${sandbox}.sandboxes.test`,
    port: AGENT_ROUTE_PORT,
    front: 'http',
    publicPort: 80,
    epoch: 1,
    destination: `127.0.0.1:${agent.port}`,
    agent: {
      manifest: {
        portRevision: 0,
        exposed: [],
        activityAt: 0,
        activeStreams: 0,
        publicKey: publicEncoded,
        port: { port: 2, hostPort: agent.port as number, endpoints: [`127.0.0.1:${agent.port}`] },
        spares: [{ port: 1, hostPort: app.port as number, endpoints: [`127.0.0.1:${app.port}`] }],
      },
    },
  }
  const token = (changes: Record<string, unknown> = {}) => {
    const now = Math.floor(Date.now() / SECOND_MS)
    const payload = {
      sandbox,
      epoch: 1,
      scope: 'sandbox',
      issuedAt: now,
      expiresAt: now + node['x-pryx-agent'].tokenLifetimeSeconds,
      maxPorts: 1,
      ...changes,
    }
    const text = `${node['x-pryx-agent'].tokenPrefix}.${Buffer.from(JSON.stringify(payload)).toString('base64url')}`
    return `${text}.${sign(null, Buffer.from(text), privateKey).toString('base64url')}`
  }
  if (route.agent) route.agent.routeToken = token({ scope: 'routes' })
  table.apply({ routes: [route] })
  const hold = new Hold(async () => {
    wakes++
    table.apply({ routes: [{ ...route, state: undefined }] })
    return { outcome: 'ready', destination: route.destination, epoch: route.epoch }
  })
  const proxy = new SandboxProxy(table, hold, 5 * SECOND_MS)
  return {
    route,
    table,
    proxy,
    token,
    result,
    calls,
    frames,
    output,
    logFrames,
    get wakes() {
      return wakes
    },
    url: `http://127.0.0.1:${proxy.server.port}/sandboxes/${sandbox}`,
    close() {
      proxy.close()
      agent.stop(true)
      app.stop(true)
    },
  }
}
