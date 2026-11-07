// SPDX-License-Identifier: FSL-1.1-ALv2
/** Reference MCP clients exercise real container commands and files through the CLI transports
 * and the sandbox's authenticated URL. These acceptance cases require CI's real deployment. */
import { expect, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'
import { StatusCodes } from 'http-status-codes'
import { agentDeployment } from '../../control/e2e/agent-context.ts'
import { ready } from '../../control/e2e/deployment.ts'
import { MINUTE_MS } from '../../control/src/durations.ts'
import { mcpHttp } from '../src/mcp.ts'

function value(input: unknown) {
  const answer = CallToolResultSchema.parse(input)
  expect(answer.isError).not.toBe(true)
  const text = answer.content.find((part) => part.type === 'text')
  if (!text || text.type !== 'text') throw new Error('MCP result omitted text')
  return JSON.parse(text.text)
}
test(
  'reference MCP clients create, execute and read through both CLI transports',
  async () => {
    if (!ready) throw new Error('MCP acceptance requires the real CI deployment')
    const c = await agentDeployment(),
      http = mcpHttp(c.sdk, 0)
    try {
      for (const transport of [
        new StdioClientTransport({
          command: process.env.PRYX_MCP_CLI ?? process.execPath,
          args: process.env.PRYX_MCP_CLI ? ['mcp'] : ['cli/src/main.ts', 'mcp'],
          env: Object.fromEntries(
            Object.entries(c.d.env).filter(
              (entry): entry is [string, string] => typeof entry[1] === 'string',
            ),
          ),
          stderr: 'inherit',
        }),
        new StreamableHTTPClientTransport(new URL('/mcp', http.url), {
          requestInit: { headers: { authorization: `Bearer ${c.d.credential.key}` } },
        }),
      ]) {
        const client = new Client({ name: 'pryx-ci', version: '1.0.0' })
        let id: string | undefined
        try {
          await client.connect(transport)
          const made = value(
            await client.callTool({ name: 'sandbox_create', arguments: { image: 'alpine:pryx-tls' } }),
          )
          id = made.id
          const run = value(
            await client.callTool({
              name: 'sandbox_run',
              arguments: { id, command: 'printf reference > /data/mcp-file; printf mcp-ready' },
            }),
          )
          expect(run.code).toBe(0)
          expect(run.stdout).toBe('mcp-ready')
          expect(
            value(await client.callTool({ name: 'sandbox_read', arguments: { id, path: 'mcp-file' } })).text,
          ).toBe('reference')
          value(
            await client.callTool({
              name: 'sandbox_write',
              arguments: { id, path: 'written', text: 'saved' },
            }),
          )
          expect(
            value(await client.callTool({ name: 'sandbox_list_files', arguments: { id, path: '.' } })).some(
              (file: { name: string }) => file.name === 'written',
            ),
          ).toBe(true)
          const state = await client.readResource({ uri: `pryx://sandboxes/${id}/state` })
          const first = state.contents[0]
          if (!first || !('text' in first)) throw new Error('State resource omitted text')
          expect(JSON.parse(first.text).id).toBe(id)
          await client.readResource({ uri: `pryx://sandboxes/${id}/logs` })
          value(await client.callTool({ name: 'sandbox_sleep', arguments: { id } }))
          value(await client.callTool({ name: 'sandbox_wake', arguments: { id } }))
          const snapshot = value(await client.callTool({ name: 'sandbox_snapshot', arguments: { id } }))
          const twin = value(
            await client.callTool({ name: 'sandbox_fork', arguments: { snapshot: snapshot.id } }),
          )
          value(await client.callTool({ name: 'sandbox_delete', arguments: { id: twin.id } }))
          await c.sdk.api.deleteSnapshot(id as string, snapshot.id)
          expect(value(await client.callTool({ name: 'project_usage', arguments: {} })).usage).toBeDefined()
        } finally {
          if (id) await c.sdk.api.delete(id)
          await client.close()
        }
      }
    } finally {
      http.stop(true)
      await c.close()
    }
  },
  4 * MINUTE_MS,
)

test(
  'a sandbox MCP URL runs commands and keeps file and sandbox authority',
  async () => {
    if (!ready) throw new Error('Sandbox MCP acceptance requires the real CI deployment')
    const c = await agentDeployment(),
      box = await c.sdk.sandbox({ image: 'alpine:pryx-tls' })
    const client = new Client({ name: 'pryx-sandbox-ci', version: '1.0.0' })
    try {
      await c.route(box.id)
      const url = new URL(await box.mcpUrl())
      const unauthenticated = new URL(url)
      unauthenticated.search = ''
      expect(
        (
          await fetch(unauthenticated, {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: '{}',
          })
        ).status,
      ).toBe(StatusCodes.UNAUTHORIZED)
      await client.connect(new StreamableHTTPClientTransport(url))
      expect((await client.listTools()).tools.some((tool) => tool.name === 'run')).toBe(true)
      expect(
        value(await client.callTool({ name: 'run', arguments: { command: 'printf sandbox-mcp' } })).stdout,
      ).toBe('sandbox-mcp')
      value(await client.callTool({ name: 'write', arguments: { path: 'mcp.txt', text: 'local' } }))
      expect(value(await client.callTool({ name: 'read', arguments: { path: 'mcp.txt' } })).text).toBe(
        'local',
      )
      expect((await client.callTool({ name: 'read', arguments: { path: '../outside' } })).isError).toBe(true)
      const started = value(await client.callTool({ name: 'start', arguments: { command: 'sleep 60' } }))
      value(await client.callTool({ name: 'process_signal', arguments: { id: started.id, signal: 'TERM' } }))
      expect(
        value(await client.callTool({ name: 'process_wait', arguments: { id: started.id } })).code,
      ).not.toBe(0)
      await client.readResource({ uri: 'pryx://sandbox/state' })
      await client.readResource({ uri: 'pryx://sandbox/logs' })
    } finally {
      await client.close()
      await box.delete()
      await c.close()
    }
  },
  2 * MINUTE_MS,
)
