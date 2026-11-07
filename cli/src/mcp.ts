// SPDX-License-Identifier: FSL-1.1-ALv2
/** Optional stdio and stateless Streamable HTTP transports. HTTP binds loopback, validates
 * Host/Origin and authenticates every request with the configured project key. */
import { timingSafeEqual } from 'node:crypto'
import type { Pryx } from '@cubepals/pryx'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { WebStandardStreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js'
import { StatusCodes } from 'http-status-codes'
import { mcpServer } from './mcp-tools.ts'
import { option, publicClient, publicCredentials } from './sandbox.ts'

export function mcpHttp(client: Pryx, port: number, key: string) {
  const expected = Buffer.from(`Bearer ${key}`)
  return Bun.serve({
    hostname: '127.0.0.1',
    port,
    maxRequestBodySize: 2 * 1024 * 1024,
    async fetch(request, listener) {
      const url = new URL(request.url)
      const hosts = [`127.0.0.1:${listener.port}`, `localhost:${listener.port}`]
      if (
        !hosts.includes(request.headers.get('host') ?? url.host) ||
        (request.headers.has('origin') &&
          !hosts.some((host) => request.headers.get('origin') === `http://${host}`))
      )
        return new Response('Origin refused', { status: StatusCodes.FORBIDDEN })
      const given = Buffer.from(request.headers.get('authorization') ?? '')
      if (given.length !== expected.length || !timingSafeEqual(given, expected))
        return new Response('Project key required', { status: StatusCodes.UNAUTHORIZED })
      if (url.pathname !== '/mcp') return new Response(null, { status: StatusCodes.NOT_FOUND })
      const transport = new WebStandardStreamableHTTPServerTransport({
        sessionIdGenerator: undefined,
        enableJsonResponse: true,
        maxRequestBodySize: 2 * 1024 * 1024,
      })
      const server = mcpServer(client)
      await server.connect(transport)
      try {
        return await transport.handleRequest(request)
      } finally {
        await server.close()
      }
    },
    error: () => new Response('MCP request failed', { status: StatusCodes.INTERNAL_SERVER_ERROR }),
  })
}
export async function mcp(args: string[]) {
  const client = publicClient(),
    http = option(args, '--http')
  if (http !== undefined) {
    const port = Number(http)
    if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('--http needs a TCP port')
    const server = mcpHttp(client, port, publicCredentials().key)
    console.error(`pryx MCP listening on http://127.0.0.1:${server.port}/mcp`)
  } else {
    await mcpServer(client).connect(new StdioServerTransport())
  }
}
