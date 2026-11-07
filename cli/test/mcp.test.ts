// SPDX-License-Identifier: FSL-1.1-ALv2
/** The reference MCP client exercises both CLI transports against a real control database.
 * Container execution and the sandbox-local MCP endpoint are mandatory CI acceptance cases. */
import { expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { StatusCodes } from 'http-status-codes'
import { publicContext } from '../../control/test/public-context.ts'
import { mcpHttp } from '../src/mcp.ts'

test('reference MCP clients use CLI HTTP and stdio with project authority', async () => {
  const c = await publicContext()
  const api = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: (request) => c.app.fetch(request) })
  const sdk = new Pryx({ url: api.url.toString(), key: c.credential.key })
  const http = mcpHttp(sdk, 0, c.credential.key)
  try {
    const url = new URL('/mcp', http.url)
    expect((await fetch(url)).status).toBe(StatusCodes.UNAUTHORIZED)
    expect(
      (
        await fetch(url, {
          headers: { authorization: `Bearer ${c.credential.key}`, origin: 'https://other.test' },
        })
      ).status,
    ).toBe(StatusCodes.FORBIDDEN)
    const transports = [
      new StreamableHTTPClientTransport(url, {
        requestInit: { headers: { authorization: `Bearer ${c.credential.key}` } },
      }),
      new StdioClientTransport({
        command: process.execPath,
        args: ['cli/src/main.ts', 'mcp'],
        env: { PRYX_API_URL: api.url.toString(), PRYX_API_KEY: c.credential.key },
        stderr: 'pipe',
      }),
    ]
    for (const transport of transports) {
      const client = new Client({ name: 'pryx-reference-acceptance', version: '1.0.0' })
      try {
        await client.connect(transport)
        const tools = await client.listTools()
        expect(tools.tools.map((tool) => tool.name)).toContain('project_usage')
        expect(tools.tools).toHaveLength(15)
        const created = await client.callTool({ name: 'sandbox_create', arguments: { image: 'alpine:3.22' } })
        expect(created.isError).not.toBe(true)
        const content = created.content as { type: string; text: string }[]
        const box = JSON.parse(content[0]?.text ?? '{}') as { id: string }
        expect(box.id).toBeString()
        const resource = await client.readResource({ uri: `pryx://sandboxes/${box.id}/state` })
        const first = resource.contents[0]
        if (!first || !('text' in first)) throw new Error('State resource omitted text')
        expect(JSON.parse(first.text).id).toBe(box.id)
        const snapshot = await client.callTool({ name: 'sandbox_snapshot', arguments: { id: box.id } })
        expect(snapshot.isError).not.toBe(true)
        const snapshotContent = snapshot.content as { type: string; text: string }[]
        const saved = JSON.parse(snapshotContent[0]?.text ?? '{}') as { id: string }
        expect(saved.id).toBeString()
        const fork = await client.callTool({ name: 'sandbox_fork', arguments: { snapshot: saved.id } })
        expect(fork.isError).not.toBe(true)
        const forkContent = fork.content as { type: string; text: string }[]
        const child = JSON.parse(forkContent[0]?.text ?? '{}') as { id: string }
        expect(child.id).toBeString()
        expect(
          (await client.callTool({ name: 'sandbox_delete', arguments: { id: child.id } })).isError,
        ).not.toBe(true)
        expect(
          (await client.callTool({ name: 'sandbox_delete', arguments: { id: box.id } })).isError,
        ).not.toBe(true)
      } finally {
        await client.close()
      }
    }
  } finally {
    http.stop(true)
    api.stop(true)
    await c.close()
  }
})
