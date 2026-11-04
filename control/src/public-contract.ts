// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** OpenAPI operations composed from the runtime zod schemas, including the streaming event unions. */
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import {
  errorSchema,
  execEvent,
  execSchema,
  makeSchema,
  sandboxSchema,
  snapshotSchema,
  templateSchema,
} from './api-schemas.ts'

export function publicContract() {
  const definitions = {
    Sandbox: sandboxSchema,
    MakeRequest: makeSchema,
    ExecRequest: execSchema,
    ExecEvent: execEvent,
    Snapshot: snapshotSchema,
    Error: errorSchema,
    TemplateRequest: templateSchema,
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
      ...(body ? { requestBody: { required: true, content: { 'application/json': { schema: body } } } } : {}),
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
  operation('post', '/v1/sandboxes', ref('Sandbox'), ref('MakeRequest'), StatusCodes.CREATED)
  operation('get', '/v1/sandboxes', array('sandboxes', ref('Sandbox')))
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
    ['since'],
  )
  operation('post', '/v1/templates', object, ref('TemplateRequest'), StatusCodes.CREATED)
  operation('get', '/v1/templates', array('templates', object))
  return {
    openapi: '3.1.0',
    info: { title: 'pryx public API', version: '1.0.0' },
    paths,
    components: { schemas: components, securitySchemes: { apiKey: { type: 'http', scheme: 'bearer' } } },
  }
}
