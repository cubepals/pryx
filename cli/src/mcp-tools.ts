// SPDX-License-Identifier: FSL-1.1-ALv2
/** Optional MCP tools and resources over the public SDK. This adapter owns no placement,
 * credentials, filesystem access or retry loop; project admission stays in the public API. */
import type { Pryx } from '@cubepals/pryx'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import {
  CallToolRequestSchema,
  ErrorCode,
  ListResourcesRequestSchema,
  ListResourceTemplatesRequestSchema,
  ListToolsRequestSchema,
  McpError,
  ReadResourceRequestSchema,
} from '@modelcontextprotocol/sdk/types.js'
import { z } from 'zod/v3'
import { zodToJsonSchema } from 'zod-to-json-schema'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'

const id = z.string().min(1).max(128)
const path = z.string().max(4096)
const FILE_BYTES = 1024 * 1024
const command = z.union([z.string().min(1).max(FILE_BYTES), z.array(z.string()).min(1).max(256)])
const text = (value: unknown) => ({
  content: [{ type: 'text' as const, text: JSON.stringify(value ?? null) }],
})
async function answer(operation: () => Promise<unknown>) {
  try {
    return text(await operation())
  } catch (error) {
    return { ...text({ error: error instanceof Error ? error.message : 'Operation failed' }), isError: true }
  }
}
async function read(client: Pryx, sandbox: string, file: string) {
  const reader = (await (await client.sandbox(sandbox, { wake: false })).files.readStream(file)).getReader()
  const decoder = new TextDecoder('utf8', { fatal: true })
  let size = 0,
    value = ''
  try {
    for (;;) {
      const part = await reader.read()
      if (part.done) return value + decoder.decode()
      size += part.value.byteLength
      if (size > FILE_BYTES) throw new Error('MCP file reads are limited to 1 MiB; use the SDK for streaming')
      value += decoder.decode(part.value, { stream: true })
    }
  } finally {
    await reader.cancel()
  }
}
export function mcpServer(client: Pryx) {
  const server = new Server(
    { name: 'pryx', version: PRYX_VERSION },
    { capabilities: { tools: {}, resources: {} } },
  )
  const tools = new Map<
    string,
    { description: string; schema: z.AnyZodObject; call: (args: unknown) => ReturnType<typeof answer> }
  >()
  function registerTool<T extends z.ZodRawShape>(
    name: string,
    options: { description: string; inputSchema: T },
    handler: (args: z.infer<z.ZodObject<T>>) => ReturnType<typeof answer>,
  ) {
    const schema = z.object(options.inputSchema).strict()
    tools.set(name, { description: options.description, schema, call: (args) => handler(schema.parse(args)) })
  }
  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [...tools].map(([name, tool]) => ({
      name,
      description: tool.description,
      inputSchema: {
        ...zodToJsonSchema(tool.schema, { target: 'jsonSchema7', $refStrategy: 'none' }),
        type: 'object' as const,
      },
    })),
  }))
  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const tool = tools.get(request.params.name)
    if (!tool) throw new McpError(ErrorCode.InvalidParams, 'Unknown tool')
    try {
      return await tool.call(request.params.arguments ?? {})
    } catch (error) {
      return {
        ...text({ error: error instanceof z.ZodError ? 'Invalid tool arguments' : 'Tool failed' }),
        isError: true,
      }
    }
  })
  registerTool(
    'sandbox_create',
    {
      description: 'Create a sandbox from an image, template or the process userland',
      inputSchema: {
        image: z.string().optional(),
        template: z.string().optional(),
        units: z.number().int().positive().optional(),
        key: z.string().optional(),
      },
    },
    ({ key, ...options }) => answer(async () => client.sandbox({ ...options, idempotencyKey: key })),
  )
  registerTool('sandbox_list', { description: 'List sandboxes in this project', inputSchema: {} }, () =>
    answer(async () => ({ sandboxes: await client.sandboxes() })),
  )
  registerTool('sandbox_get', { description: 'Read sandbox state', inputSchema: { id } }, ({ id }) =>
    answer(() => client.sandbox(id, { wake: false })),
  )
  registerTool(
    'sandbox_delete',
    { description: 'Permanently delete a sandbox and its data', inputSchema: { id } },
    ({ id }) => answer(() => (async () => (await client.sandbox(id, { wake: false })).delete())()),
  )
  const run = { id, command, timeout: z.number().int().positive().optional(), key: z.string().optional() }
  registerTool(
    'sandbox_run',
    { description: 'Run a command and collect its bounded output', inputSchema: run },
    ({ id, command, timeout, key }) =>
      answer(async () => (await client.sandbox(id)).run(command, { timeout, idempotencyKey: key })),
  )
  registerTool(
    'sandbox_start',
    { description: 'Start a background process and return its ID', inputSchema: run },
    ({ id, command, timeout, key }) =>
      answer(
        async () => (await (await client.sandbox(id)).spawn(command, { timeout, idempotencyKey: key })).view,
      ),
  )
  registerTool(
    'sandbox_read',
    { description: 'Read a UTF-8 file up to 1 MiB', inputSchema: { id, path } },
    ({ id, path }) => answer(async () => ({ text: await read(client, id, path) })),
  )
  registerTool(
    'sandbox_write',
    { description: 'Write a UTF-8 file', inputSchema: { id, path, text: z.string().max(FILE_BYTES) } },
    ({ id, path, text }) => answer(async () => (await client.sandbox(id, { wake: false })).write(path, text)),
  )
  registerTool(
    'sandbox_list_files',
    { description: 'List files in a sandbox directory', inputSchema: { id, path: path.default('.') } },
    ({ id, path }) => answer(async () => (await client.sandbox(id, { wake: false })).files.list(path)),
  )
  registerTool('sandbox_snapshot', { description: 'Snapshot sandbox data', inputSchema: { id } }, ({ id }) =>
    answer(() => (async () => (await client.sandbox(id, { wake: false })).snapshot())()),
  )
  registerTool(
    'sandbox_fork',
    {
      description: 'Fork a snapshot into a new sandbox',
      inputSchema: { snapshot: id, key: z.string().optional() },
    },
    ({ snapshot, key }) =>
      answer(() => client.sandbox({ snapshot, idempotencyKey: key ?? crypto.randomUUID() })),
  )
  for (const action of ['sleep', 'wake'] as const)
    registerTool(`sandbox_${action}`, { description: `${action} a sandbox`, inputSchema: { id } }, ({ id }) =>
      answer(() =>
        (async () => ({ sandbox: await (await client.sandbox(id, { wake: false }))[action]() }))(),
      ),
    )
  registerTool(
    'sandbox_expose',
    {
      description: 'Expose an application using a preallocated spare port',
      inputSchema: { id, port: z.number().int().min(1).max(65535) },
    },
    ({ id, port }) => answer(async () => (await client.sandbox(id)).expose(port)),
  )
  registerTool(
    'project_usage',
    { description: 'Read this project’s usage and limits', inputSchema: {} },
    () => answer(() => client.project()),
  )
  server.setRequestHandler(ListResourceTemplatesRequestSchema, async () => ({
    resourceTemplates: ['state', 'logs'].map((kind) => ({
      name: `sandbox_${kind}`,
      uriTemplate: `pryx://sandboxes/{id}/${kind}`,
      mimeType: 'application/json',
    })),
  }))
  server.setRequestHandler(ListResourcesRequestSchema, async () => ({
    resources: (await client.sandboxes()).flatMap((box) =>
      ['state', 'logs'].map((kind) => ({
        name: `${box.id} ${kind}`,
        uri: `pryx://sandboxes/${box.id}/${kind}`,
        mimeType: 'application/json',
      })),
    ),
  }))
  server.setRequestHandler(ReadResourceRequestSchema, async (request) => {
    const url = new URL(request.params.uri),
      match = /^\/([^/]+)\/(state|logs)$/.exec(url.pathname)
    if (url.protocol !== 'pryx:' || url.hostname !== 'sandboxes' || !match)
      throw new McpError(ErrorCode.InvalidParams, 'Unknown resource')
    const sandbox = match[1] as string
    return {
      contents: [
        {
          uri: url.href,
          mimeType: 'application/json',
          text: JSON.stringify(
            match[2] === 'state'
              ? await client.sandbox(sandbox, { wake: false })
              : await Array.fromAsync((await client.sandbox(sandbox)).logs({ follow: false, tail: 200 })),
          ),
        },
      ],
    }
  })
  return server
}
