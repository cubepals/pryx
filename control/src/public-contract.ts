// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** OpenAPI operations composed from the runtime zod schemas, including the streaming event unions. */
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import { sandboxTokenSchema } from './agents.ts'
import {
  errorSchema,
  execEvent,
  execSchema,
  makeSchema,
  networkSchema,
  sandboxSchema,
  snapshotSchema,
  templateSchema,
} from './api-schemas.ts'
import { versionSchema } from './code-templates.ts'
import { chunkWrite } from './file-chunks.ts'

import { sshAddress, sshRequest } from './native-ssh.ts'
import { agentSchema, fileInfoSchema } from './node-contract.ts'
import { limitsSchema } from './project-model.ts'
import { lifetimeSchema } from './sandbox-lifetime.ts'
import { SDK_DEFAULTS } from './sdk-defaults.ts'
import { webhookCreatedSchema, webhookSchema, webhookViewSchema } from './webhooks.ts'

export function publicContract() {
  const definitions = {
    SshRegister: sshRequest,
    SshAddress: sshAddress,
    Sandbox: sandboxSchema,
    SandboxToken: sandboxTokenSchema,
    FileInfo: fileInfoSchema,
    AgentStart: agentSchema('Start'),
    AgentProcess: agentSchema('View'),
    AgentResult: agentSchema('ResultView'),
    AgentEvent: agentSchema('Event'),
    AgentExposed: agentSchema('Exposed'),
    AgentLogFrame: agentSchema('Frame'),
    MakeRequest: makeSchema,
    ExecRequest: execSchema,
    ExecEvent: execEvent,
    Snapshot: snapshotSchema,
    Error: errorSchema,
    TemplateRequest: templateSchema,
    LifetimeRequest: lifetimeSchema,
    TemplateVersionRequest: versionSchema,
    WebhookRequest: webhookSchema,
    Webhook: webhookViewSchema,
    WebhookCreated: webhookCreatedSchema,
    Project: z.object({
      defaults: z
        .object({ isolation: z.enum(['container', 'process', 'wasm']), repoImage: z.string() })
        .optional(),
      id: z.string(),
      name: z.string(),
      state: z.enum(['active', 'frozen', 'deleted']),
      createdAt: z.string(),
      limits: limitsSchema,
      usage: z.object({
        units: z.number(),
        sandboxes: z.number(),
        running: z.number(),
        snapshots: z.number(),
        snapshotBytes: z.number(),
        egressPorts: z.number(),
      }),
    }),
  }
  const json = (schema: z.ZodType) => {
    const { $schema: _, ...result } = z.toJSONSchema(schema, { unrepresentable: 'any', io: 'input' })
    return result
  }
  const components = Object.fromEntries(Object.entries(definitions).map(([k, v]) => [k, json(v)]))
  const ref = (name: string) => ({ $ref: `#/components/schemas/${name}` })
  const paths: Record<string, Record<string, unknown>> = {}
  const operation = (
    method: string,
    path: string,
    response: unknown,
    body?: unknown,
    status: number = StatusCodes.OK,
    content = 'application/json',
    query: string[] = [],
    bodyContent = 'application/json',
  ) => {
    const parameters = [
      ...Array.from(path.matchAll(/\{(\w+)\}/g), (m) => ({
        name: m[1],
        in: 'path',
        required: true,
        schema: { type: 'string' },
      })),
      ...query.map((name) => ({ name, in: 'query', required: false, schema: { type: 'string' } })),
    ]
    if (method === 'post' && (path.endsWith('/exec') || path.endsWith('/sandboxes')))
      parameters.push({ name: 'Idempotency-Key', in: 'header', required: false, schema: { type: 'string' } })
    const errors = Object.fromEntries(
      [
        StatusCodes.UNAUTHORIZED,
        StatusCodes.FORBIDDEN,
        StatusCodes.NOT_FOUND,
        StatusCodes.GONE,
        StatusCodes.CONFLICT,
        StatusCodes.UNPROCESSABLE_ENTITY,
        StatusCodes.SERVICE_UNAVAILABLE,
      ].map((s) => [
        s,
        { description: 'Request refused', content: { 'application/json': { schema: ref('Error') } } },
      ]),
    )
    const entry = paths[path] ?? {}
    paths[path] = entry
    entry[method] = {
      operationId: `${method}_${path.replaceAll(/[^a-zA-Z0-9]+/g, '_')}`,
      security: [{ apiKey: [] }],
      parameters,
      ...(body ? { requestBody: { required: true, content: { [bodyContent]: { schema: body } } } } : {}),
      responses: {
        ...errors,
        [status]: { description: 'Success', content: { [content]: { schema: response } } },
      },
    }
  }
  const array = (key: string, schema: unknown) => ({
    type: 'object',
    required: [key],
    properties: { [key]: { type: 'array', items: schema } },
  })
  const object = { type: 'object', additionalProperties: true }
  const existed = { type: 'object', required: ['existed'], properties: { existed: { type: 'boolean' } } }
  operation('get', '/v1/project', ref('Project'))
  operation('post', '/v1/sandboxes/{id}/ssh', ref('SshAddress'), ref('SshRegister'))
  operation('delete', '/v1/sandboxes/{id}/ssh', {
    type: 'object',
    properties: { revoked: { type: 'boolean' } },
    required: ['revoked'],
  })
  operation('post', '/v1/sandboxes/{id}/token', ref('SandboxToken'))
  operation(
    'post',
    '/v1/sandboxes/{id}/network',
    ref('Sandbox'),
    json(z.object({ network: networkSchema }).strict()),
  )
  operation(
    'get',
    '/v1/sandboxes/{id}/data/content',
    { type: 'string', format: 'binary' },
    undefined,
    StatusCodes.OK,
    'application/octet-stream',
    ['path'],
  )
  operation(
    'get',
    '/v1/sandboxes/{id}/data/list',
    array('files', ref('FileInfo')),
    undefined,
    StatusCodes.OK,
    'application/json',
    ['path'],
  )
  operation(
    'get',
    '/v1/sandboxes/{id}/data/stat',
    ref('FileInfo'),
    undefined,
    StatusCodes.OK,
    'application/json',
    ['path'],
  )
  operation(
    'put',
    '/v1/sandboxes/{id}/data/content',
    {},
    { type: 'string', format: 'binary' },
    StatusCodes.NO_CONTENT,
    'application/json',
    ['path'],
    'application/octet-stream',
  )
  operation('post', '/v1/sandboxes', ref('Sandbox'), ref('MakeRequest'), StatusCodes.CREATED)
  operation(
    'get',
    '/v1/sandboxes',
    array('sandboxes', ref('Sandbox')),
    undefined,
    StatusCodes.OK,
    'application/json',
    ['labels', 'state', 'name'],
  )
  operation('post', '/v1/sandboxes/{id}/lifetime', ref('Sandbox'), ref('LifetimeRequest'))
  operation('get', '/v1/sandboxes/{id}/metrics', object, undefined, StatusCodes.OK, 'application/json', [
    'since',
  ])
  operation(
    'post',
    '/v1/templates/{name}/versions',
    object,
    ref('TemplateVersionRequest'),
    StatusCodes.CREATED,
  )
  operation('get', '/v1/templates/{name}/versions', array('versions', object))
  operation('post', '/v1/webhooks', ref('WebhookCreated'), ref('WebhookRequest'), StatusCodes.CREATED)
  operation('get', '/v1/webhooks', array('webhooks', ref('Webhook')))
  operation('delete', '/v1/webhooks/{id}', existed)
  operation('get', '/v1/webhooks/{id}/deadletters', array('deliveries', object))
  operation('get', '/v1/sandboxes/{id}', ref('Sandbox'))
  operation('delete', '/v1/sandboxes/{id}', existed)
  for (const verb of ['start', 'stop', 'sleep', 'wake', 'rest'])
    operation('post', `/v1/sandboxes/{id}/${verb}`, {
      type: 'object',
      required: ['sandbox'],
      properties: {
        sandbox: ref('Sandbox'),
        tier: { type: 'string', enum: ['suspended', 'stopped', 'resumed', 'booted', 'restored'] },
      },
    })
  operation(
    'post',
    '/v1/sandboxes/{id}/exec',
    ref('ExecEvent'),
    ref('ExecRequest'),
    StatusCodes.OK,
    'application/x-ndjson',
  )
  operation(
    'get',
    '/v1/sandboxes/{id}/logs',
    { type: 'object', additionalProperties: true },
    undefined,
    StatusCodes.OK,
    'application/x-ndjson',
    ['tail', 'since', 'follow'],
  )
  operation('put', '/v1/sandboxes/{id}/files/{path}', object, {
    type: 'object',
    required: ['dataBase64'],
    properties: { dataBase64: { type: 'string', contentEncoding: 'base64' } },
  })
  operation('get', '/v1/sandboxes/{id}/files/{path}', {
    type: 'object',
    required: ['archiveBase64'],
    properties: { archiveBase64: { type: 'string', contentEncoding: 'base64' } },
  })
  operation('post', '/v1/sandboxes/{id}/snapshots', ref('Snapshot'), undefined, StatusCodes.CREATED)
  operation('put', '/v1/sandboxes/{id}/file-chunks/{path}', object, json(chunkWrite))
  operation(
    'get',
    '/v1/sandboxes/{id}/file-chunks/{path}',
    {
      type: 'object',
      required: ['archiveBase64'],
      properties: { archiveBase64: { type: 'string', contentEncoding: 'base64' } },
    },
    undefined,
    StatusCodes.OK,
    'application/json',
    ['offset'],
  )
  operation('get', '/v1/sandboxes/{id}/snapshots', array('snapshots', ref('Snapshot')))
  operation('delete', '/v1/sandboxes/{id}/snapshots/{snapshot}', existed)
  operation(
    'post',
    '/v1/snapshots/{snapshot}/sandboxes',
    ref('Sandbox'),
    ref('MakeRequest'),
    StatusCodes.CREATED,
  )
  operation('get', '/v1/sandboxes/{id}/endpoints', {
    type: 'object',
    required: ['endpoints'],
    properties: { endpoints: json(sandboxSchema.shape.endpoints) },
  })
  operation(
    'get',
    '/v1/sandboxes/{id}/events',
    array('events', object),
    undefined,
    StatusCodes.OK,
    'application/json',
    ['since', 'stream', 'follow'],
  )
  operation('post', '/v1/templates', object, ref('TemplateRequest'), StatusCodes.CREATED)
  operation('get', '/v1/templates', array('templates', object))
  return {
    openapi: '3.1.0',
    info: { title: 'pryx public API', version: '1.0.0' },
    'x-pryx-sdk': SDK_DEFAULTS,
    paths,
    components: { schemas: components, securitySchemes: { apiKey: { type: 'http', scheme: 'bearer' } } },
  }
}
