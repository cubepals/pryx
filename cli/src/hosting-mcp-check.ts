// SPDX-License-Identifier: FSL-1.1-ALv2
/** Run the exact welcome-page MCP configuration with the reference client, then clean up.
 * Invoked by deployment acceptance only; not part of the CLI's public commands. */
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'

const configuration = JSON.parse(await new Response(Bun.stdin.stream()).text()).mcpServers.pryx
const client = new Client({ name: 'pryx-hosting-ci', version: '1.0.0' })
const transport = new StdioClientTransport({
  ...configuration,
  env: { ...process.env, ...configuration.env },
  stderr: 'pipe',
})
const result = (value: unknown) => {
  const parsed = CallToolResultSchema.parse(value)
  if (parsed.isError) throw new Error('Welcome MCP tool failed')
  const item = parsed.content.find((entry) => entry.type === 'text')
  if (!item || item.type !== 'text') throw new Error('Welcome MCP omitted result')
  return JSON.parse(item.text)
}
let id: string | undefined
try {
  await client.connect(transport)
  id = result(await client.callTool({ name: 'sandbox_create', arguments: {} })).id
  const run = result(await client.callTool({ name: 'sandbox_run', arguments: { id, command: 'echo hello' } }))
  if (run.code !== 0 || run.stdout.trim() !== 'hello') throw new Error('Welcome MCP hello differs')
} finally {
  if (id) result(await client.callTool({ name: 'sandbox_delete', arguments: { id } }))
  await client.close()
}
console.log('hello')
