// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Authenticated public v1 routes. All placement and lifecycle work goes through the sandbox model. */
import { Hono } from 'hono'
import { stream } from 'hono/streaming'
import type { ContentfulStatusCode } from 'hono/utils/http-status'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import { execSchema, makeSchema, templateSchema } from './api-schemas.ts'
import type { Db } from './database.ts'
import { events } from './events.ts'
import { chunkOffset, chunkWrite, FileChunks } from './file-chunks.ts'
import { authenticate } from './keys.ts'
import { type NodeClient, NodeRefused } from './node-client.ts'
import { PublicModel } from './public-model.ts'
import { SandboxIO } from './sandbox-io.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'

export function createPublicApi(options: {
  db: Db
  boxes: Sandboxes
  nodes: NodeClient
  domain: string
  edgePortRange?: readonly [number, number]
}) {
  const { db, boxes, nodes, domain } = options
  const model = new PublicModel(db, boxes, domain, options.edgePortRange)
  const io = new SandboxIO(db, boxes, nodes)
  const chunks = new FileChunks(io)
  const app = new Hono<{ Variables: { project: string }; Bindings: { beginStream?: () => void } }>()
  app.use('/v1/*', async (c, next) => {
    const auth = c.req.header('authorization') ?? ''
    const identity = auth.startsWith('Bearer ') ? await authenticate(db, auth.slice('Bearer '.length)) : null
    if (!identity)
      return c.json(
        { error: { code: 'unauthorized', message: 'A valid API key is required' } },
        StatusCodes.UNAUTHORIZED,
      )
    c.set('project', identity.project)
    await next()
  })
  app.onError((error, c) => {
    if (error instanceof SyntaxError)
      return c.json(
        { error: { code: 'invalid_request', message: 'Invalid JSON body' } },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    if (error instanceof z.ZodError)
      return c.json(
        {
          error: {
            code: 'invalid_request',
            message: 'Invalid request',
            details: error.issues.map((i) => ({ field: i.path.join('.'), problem: i.message })),
          },
        },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    const code = error instanceof SandboxError || error instanceof NodeRefused ? error.code : 'no_room'
    const status = ({
      invalid_request: StatusCodes.UNPROCESSABLE_ENTITY,
      idempotency_key_reused: StatusCodes.UNPROCESSABLE_ENTITY,
      not_found: StatusCodes.NOT_FOUND,
      gone: StatusCodes.GONE,
      not_running: StatusCodes.CONFLICT,
      not_stopped: StatusCodes.CONFLICT,
      operation_unknown: StatusCodes.CONFLICT,
      forbidden: StatusCodes.FORBIDDEN,
    }[code] ?? StatusCodes.SERVICE_UNAVAILABLE) as ContentfulStatusCode
    return c.json(
      {
        error: {
          code,
          message:
            error instanceof SandboxError
              ? error.message
              : code === 'no_room'
                ? 'No node can complete this request'
                : 'Node refused the operation',
        },
      },
      status,
    )
  })
  app.post('/v1/sandboxes', async (c) =>
    c.json(
      await model.create(
        c.get('project'),
        makeSchema.parse(await c.req.json()),
        c.req.header('idempotency-key'),
      ),
      StatusCodes.CREATED,
    ),
  )
  app.get('/v1/sandboxes', async (c) =>
    c.json({
      sandboxes: await Promise.all(
        (await boxes.list(c.get('project'))).map((b) => model.view(b.id, c.get('project'))),
      ),
    }),
  )
  app.get('/v1/sandboxes/:id', async (c) => c.json(await model.view(c.req.param('id'), c.get('project'))))
  app.delete('/v1/sandboxes/:id', async (c) =>
    c.json(await boxes.delete(c.req.param('id'), c.get('project'))),
  )
  for (const verb of ['start', 'wake', 'stop', 'sleep', 'rest'] as const)
    app.post(`/v1/sandboxes/:id/${verb}`, async (c) => {
      const answer = await boxes[verb](c.req.param('id'), c.get('project'))
      const sandbox = await model.view(c.req.param('id'), c.get('project'))
      return c.json('tier' in answer ? { sandbox, tier: answer.tier } : { sandbox })
    })
  app.post('/v1/sandboxes/:id/exec', async (c) => {
    const body = execSchema.parse(await c.req.json())
    c.env?.beginStream?.()
    const abort = new AbortController()
    const events = await io.execEvents(
      c.req.param('id'),
      c.get('project'),
      body,
      c.req.header('idempotency-key'),
      abort.signal,
    )
    c.header('content-type', 'application/x-ndjson')
    return stream(c, async (s) => {
      s.onAbort(() => abort.abort())
      try {
        for await (const event of events) await s.writeln(JSON.stringify(event))
      } catch {
        await s.writeln(
          JSON.stringify({
            type: 'error',
            error: {
              code: 'operation_unknown',
              message: 'Exec stream ended; inspect before starting another command',
            },
          }),
        )
      }
    })
  })
  app.get('/v1/sandboxes/:id/logs', async (c) => {
    await boxes.record(c.req.param('id'), c.get('project'))
    c.env?.beginStream?.()
    c.header('content-type', 'application/x-ndjson')
    const query = new URLSearchParams()
    for (const name of ['tail', 'since', 'follow']) {
      const v = c.req.query(name)
      if (v !== undefined) query.set(name, v)
    }
    return stream(c, async (s) => {
      const abort = new AbortController()
      s.onAbort(() => abort.abort())
      try {
        for await (const line of io.logs(c.req.param('id'), c.get('project'), query, abort.signal))
          await s.writeln(line)
      } catch {
        await s.writeln(JSON.stringify({ error: { code: 'not_running', message: 'Log stream ended' } }))
      }
    })
  })
  app.get('/v1/sandboxes/:id/files/*', async (c) =>
    c.json(
      await io.file(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/files/')[1] ?? ''),
      ),
    ),
  )
  app.put('/v1/sandboxes/:id/files/*', async (c) =>
    c.json(
      await io.file(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/files/')[1] ?? ''),
        z.object({ dataBase64: z.string() }).parse(await c.req.json()).dataBase64,
      ),
    ),
  )
  app.get('/v1/sandboxes/:id/file-chunks/*', async (c) =>
    c.json(
      await chunks.transfer(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/file-chunks/')[1] ?? ''),
        chunkOffset.parse(c.req.query('offset') ?? 0),
      ),
    ),
  )
  app.put('/v1/sandboxes/:id/file-chunks/*', async (c) => {
    const body = chunkWrite.parse(await c.req.json())
    return c.json(
      await chunks.transfer(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/file-chunks/')[1] ?? ''),
        body.offset,
        body.dataBase64,
      ),
    )
  })
  app.post('/v1/sandboxes/:id/snapshots', async (c) => {
    const s = await boxes.snapshot(c.req.param('id'), c.get('project'))
    return c.json({ id: s.id, sizeBytes: s.sizeBytes, at: s.at }, StatusCodes.CREATED)
  })
  app.get('/v1/sandboxes/:id/snapshots', async (c) =>
    c.json({
      snapshots: (await boxes.snapshots(c.req.param('id'), c.get('project'))).map((s) => ({ id: s.id })),
    }),
  )
  app.delete('/v1/sandboxes/:id/snapshots/:snapshot', async (c) =>
    c.json(await boxes.deleteSnapshot(c.req.param('id'), c.req.param('snapshot'), c.get('project'))),
  )
  app.post('/v1/snapshots/:snapshot/sandboxes', async (c) =>
    c.json(
      await model.create(
        c.get('project'),
        makeSchema.parse({ ...(await c.req.json()), snapshot: c.req.param('snapshot') }),
        c.req.header('idempotency-key'),
      ),
      StatusCodes.CREATED,
    ),
  )
  app.get('/v1/sandboxes/:id/endpoints', async (c) =>
    c.json({ endpoints: (await model.view(c.req.param('id'), c.get('project'))).endpoints }),
  )
  app.get('/v1/sandboxes/:id/events', async (c) => {
    await boxes.record(c.req.param('id'), c.get('project'))
    const since = c.req.query('since')
    if (since && !Number.isFinite(Date.parse(since)))
      throw new SandboxError('invalid_request', 'since must be a timestamp')
    return c.json({ events: await events(db, c.req.param('id'), since) })
  })
  app.post('/v1/templates', async (c) => {
    const b = templateSchema.parse(await c.req.json())
    return c.json(await model.template(c.get('project'), b.name, b.spec), StatusCodes.CREATED)
  })
  app.get('/v1/templates', async (c) => c.json({ templates: await model.templates(c.get('project')) }))
  app.notFound((c) =>
    c.json({ error: { code: 'not_found', message: 'No such endpoint' } }, StatusCodes.NOT_FOUND),
  )
  return { app, model, io }
}
