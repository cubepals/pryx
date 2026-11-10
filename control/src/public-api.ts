// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Authenticated public v1 routes. All placement and lifecycle work goes through the sandbox model. */
import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import { Hono } from 'hono'
import { bodyLimit } from 'hono/body-limit'
import { stream } from 'hono/streaming'
import type { ContentfulStatusCode } from 'hono/utils/http-status'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { AgentForward } from './agent-forward.ts'
import { Agents } from './agents.ts'
import { execSchema, makeSchema, networkSchema, templateSchema } from './api-schemas.ts'
import { CodeTemplates, versionSchema } from './code-templates.ts'
import { DEFAULT_REPO_IMAGE } from './config.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { events } from './events.ts'
import { chunkOffset, chunkWrite, FileChunks } from './file-chunks.ts'
import { authenticate } from './keys.ts'
import { NativeMounts } from './native-mounts.ts'
import { NativeSsh } from './native-ssh.ts'
import { type NodeClient, NodeRefused } from './node-client.ts'
import { ProjectAdmission, projectRow, projectUsage } from './project-admission.ts'
import { exceeded, ProjectError } from './project-model.ts'
import { PublicModel } from './public-model.ts'
import { RuntimeFull } from './runtime-model.ts'
import { lifetimeSchema, SandboxLifetime } from './sandbox-lifetime.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import { one, toDate } from './sql.ts'
import { Webhooks, webhookSchema } from './webhooks.ts'

export function createPublicApi(options: {
  db: Db
  boxes: Sandboxes
  nodes: NodeClient
  domain: string
  defaultIsolation?: import('./isolation.ts').Isolation
  require?: import('./guarantees.ts').Guarantee[]
  publicOrigin?: string
  repoImage?: string
  webhookSealKey?: string
  webhookAllowHosts?: readonly string[]
  edgePortRange?: readonly [number, number]
}) {
  const { db, boxes, nodes, domain } = options
  boxes.projects ??= new ProjectAdmission(db)
  const model = new PublicModel(
    db,
    boxes,
    domain,
    options.edgePortRange,
    options.defaultIsolation,
    options.publicOrigin,
    options.require,
  )
  const agents = new Agents(boxes, nodes, domain)
  const mounts = new NativeMounts(db, options.webhookSealKey ?? '')
  agents.initializeMounts = (id, project, credential) => mounts.initialize(id, project, credential)
  model.mounts = mounts
  model.mountReady = (id, project) =>
    agents.token(id, project, options.publicOrigin ?? 'https://pryx.invalid')
  boxes.afterRunning = (id, project) => model.mountReady?.(id, project) ?? Promise.resolve()
  const ssh = new NativeSsh(db, boxes, agents, domain, options.edgePortRange)
  const codeTemplates = new CodeTemplates(db, boxes)
  const webhooks = new Webhooks(db, options.webhookSealKey ?? '', options.webhookAllowHosts)
  const lifetime = new SandboxLifetime(db, boxes)
  const io = new AgentForward(agents)
  const chunks = new FileChunks(agents)
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
    const writing = !['GET', 'HEAD', 'OPTIONS'].includes(c.req.method)
    if (writing) c.env?.beginStream?.()
    // Check resource scope before policy, so a frozen project cannot distinguish another's IDs.
    {
      const match = /^\/v1\/sandboxes\/([^/]+)(?:\/|$)/.exec(new URL(c.req.url).pathname)
      if (match) {
        const id = decodeURIComponent(match[1] as string)
        if (c.req.method === 'DELETE' && new URL(c.req.url).pathname === `/v1/sandboxes/${match[1]}`) {
          const owned = await one(
            db,
            sql`SELECT 1 FROM sandboxes WHERE id=${id} AND project=${identity.project}`,
          )
          if (!owned) throw new SandboxError('not_found', 'No such sandbox')
        } else await boxes.record(id, identity.project)
      }
      const fork = /^\/v1\/snapshots\/([^/]+)\/sandboxes$/.exec(new URL(c.req.url).pathname)
      if (fork) await boxes.snapshotRecord(decodeURIComponent(fork[1] as string), identity.project)
      const expiry = identity.expiresAt ?? boxes.projects?.defaults.expiresAt
      if (identity.expired || (expiry && Date.parse(expiry) <= Date.now()))
        exceeded('expiresAt', expiry ? toDate(expiry).toISOString() : expiry, new Date().toISOString())
      if (writing) await boxes.projects?.write(identity.project)
    }
    await next()
  })
  app.post('/v1/sandboxes/:id/ssh', async (c) =>
    c.json(await ssh.register(c.req.param('id'), c.get('project'), await c.req.json(), c.req.url)),
  )
  app.delete('/v1/sandboxes/:id/ssh', async (c) =>
    c.json(await ssh.revoke(c.req.param('id'), c.get('project'), c.req.url)),
  )
  app.onError((error, c) => {
    if (error instanceof ProjectError)
      return c.json(
        { error: { code: error.code, message: error.message, details: error.details } },
        error.code === 'not_found' ? StatusCodes.NOT_FOUND : StatusCodes.FORBIDDEN,
      )
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
      name_conflict: StatusCodes.CONFLICT,
      gone: StatusCodes.GONE,
      not_running: StatusCodes.CONFLICT,
      not_stopped: StatusCodes.CONFLICT,
      operation_unknown: StatusCodes.CONFLICT,
      forbidden: StatusCodes.FORBIDDEN,
      unsupported: StatusCodes.NOT_IMPLEMENTED,
    }[code] ?? StatusCodes.SERVICE_UNAVAILABLE) as ContentfulStatusCode
    return c.json(
      {
        error: {
          code,
          message:
            error instanceof RuntimeFull && error.missingGuarantees.length
              ? `No node provides required guarantees: ${error.missingGuarantees.join(', ')}`
              : error instanceof SandboxError
                ? error.message
                : code === 'no_room'
                  ? 'No node can complete this request'
                  : 'Node refused the operation',
        },
      },
      status,
    )
  })
  const jsonLimit = bodyLimit({ maxSize: nodeSchema['x-pryx-agent'].maxRequestBytes })
  app.use('/v1/*', (c, next) =>
    c.req.method === 'PUT' && /^\/v1\/sandboxes\/[^/]+\/data\/content$/.test(c.req.path)
      ? next()
      : jsonLimit(c, next),
  )
  app.get('/v1/project', async (c) =>
    c.json({
      ...(await projectRow(db, c.get('project'), boxes.projects?.defaults)),
      usage: await projectUsage(db, c.get('project'), boxes.projects?.unit),
      defaults: {
        isolation: options.defaultIsolation ?? 'container',
        repoImage: options.repoImage ?? DEFAULT_REPO_IMAGE,
      },
    }),
  )
  app.post('/v1/sandboxes', async (c) => {
    const body = makeSchema.parse(await c.req.json())
    c.env?.beginStream?.()
    return c.json(
      await model.create(c.get('project'), body, c.req.header('idempotency-key')),
      StatusCodes.CREATED,
    )
  })
  app.get('/v1/sandboxes', async (c) => {
    const name = c.req.query('name'),
      state = c.req.query('state')
    const raw = c.req.query('labels')
    const labels = makeSchema.shape.labels.parse(raw ? JSON.parse(raw) : {})
    const views = await Promise.all(
      (await boxes.list(c.get('project'), { name, labels })).map((box) =>
        model.view(box.id, c.get('project')),
      ),
    )
    return c.json({ sandboxes: views.filter((view) => !state || view.state === state) })
  })
  app.get('/v1/sandboxes/:id', async (c) => c.json(await model.view(c.req.param('id'), c.get('project'))))
  app.post('/v1/sandboxes/:id/lifetime', async (c) => {
    const body = lifetimeSchema.parse(await c.req.json())
    await lifetime.change(c.req.param('id'), c.get('project'), body)
    return c.json(await model.view(c.req.param('id'), c.get('project')))
  })
  app.get('/v1/sandboxes/:id/metrics', async (c) => {
    const since = c.req.query('since')
    if (since && !Number.isFinite(Date.parse(since)))
      throw new SandboxError('invalid_request', 'since must be a timestamp')
    return c.json(await lifetime.metrics(c.req.param('id'), c.get('project'), since))
  })
  app.post('/v1/sandboxes/:id/token', async (c) =>
    c.json(await agents.token(c.req.param('id'), c.get('project'), c.req.url)),
  )
  app.post('/v1/sandboxes/:id/network', async (c) => {
    const body = z
      .object({ network: networkSchema })
      .strict()
      .parse(await c.req.json())
    await boxes.network(c.req.param('id'), c.get('project'), body.network)
    return c.json(await model.view(c.req.param('id'), c.get('project')))
  })
  for (const operation of ['content', 'list', 'stat'] as const)
    app.get(`/v1/sandboxes/:id/data/${operation}`, async (c) => {
      c.env?.beginStream?.()
      return agents.data(
        c.req.param('id'),
        c.get('project'),
        operation,
        'GET',
        c.req.query('path') ?? '',
        c.req.raw.signal,
      )
    })
  app.put('/v1/sandboxes/:id/data/content', async (c) => {
    return agents.data(
      c.req.param('id'),
      c.get('project'),
      'content',
      'PUT',
      c.req.query('path') ?? '',
      c.req.raw.signal,
      c.req.raw.body ?? undefined,
    )
  })
  app.delete('/v1/sandboxes/:id', async (c) => {
    return c.json(await boxes.delete(c.req.param('id'), c.get('project')))
  })
  for (const verb of ['start', 'wake', 'stop', 'sleep', 'rest'] as const)
    app.post(`/v1/sandboxes/:id/${verb}`, async (c) => {
      const answer = await boxes[verb](c.req.param('id'), c.get('project'))
      let sandbox = await model.view(c.req.param('id'), c.get('project'))
      while ((verb === 'wake' || verb === 'start') && sandbox.state === 'starting') {
        await pause(SECOND_MS / 20)
        sandbox = await model.view(c.req.param('id'), c.get('project'))
      }
      return c.json('tier' in answer ? { sandbox, tier: answer.tier } : { sandbox })
    })
  app.post('/v1/sandboxes/:id/exec', async (c) => {
    const body = execSchema.parse(await c.req.json())
    await boxes.projects?.exec(c.get('project'), body.timeout)
    const abort = new AbortController()
    const events = await io.exec(
      c.req.param('id'),
      c.get('project'),
      body,
      c.req.header('idempotency-key'),
      abort.signal,
      c.req.url,
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
        for await (const line of io.logs(c.req.param('id'), c.get('project'), query, abort.signal, c.req.url))
          await s.writeln(line)
      } catch {
        await s.writeln(JSON.stringify({ error: { code: 'not_running', message: 'Log stream ended' } }))
      }
    })
  })
  app.get('/v1/sandboxes/:id/files/*', async (c) =>
    c.json(
      await chunks.file(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/files/')[1] ?? ''),
        undefined,
        c.req.url,
        c.req.raw.signal,
      ),
    ),
  )
  app.put('/v1/sandboxes/:id/files/*', async (c) =>
    c.json(
      await chunks.file(
        c.req.param('id'),
        c.get('project'),
        decodeURIComponent(new URL(c.req.url).pathname.split('/files/')[1] ?? ''),
        z.object({ dataBase64: z.string() }).parse(await c.req.json()).dataBase64,
        c.req.url,
        c.req.raw.signal,
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
        undefined,
        c.req.url,
        c.req.raw.signal,
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
        c.req.url,
        c.req.raw.signal,
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
  app.post('/v1/snapshots/:snapshot/sandboxes', async (c) => {
    const body = makeSchema.parse({ ...(await c.req.json()), snapshot: c.req.param('snapshot') })
    return c.json(
      await model.create(c.get('project'), body, c.req.header('idempotency-key')),
      StatusCodes.CREATED,
    )
  })
  app.get('/v1/sandboxes/:id/endpoints', async (c) =>
    c.json({ endpoints: (await model.view(c.req.param('id'), c.get('project'))).endpoints }),
  )
  app.get('/v1/sandboxes/:id/events', async (c) => {
    await boxes.record(c.req.param('id'), c.get('project'))
    const since = c.req.query('since')
    if (since && !Number.isFinite(Date.parse(since)))
      throw new SandboxError('invalid_request', 'since must be a timestamp')
    if (c.req.query('stream') === 'true') {
      c.env?.beginStream?.()
      c.header('content-type', 'application/x-ndjson')
      return stream(c, async (s) => {
        let aborted = false
        let after = '0'
        s.onAbort(() => {
          aborted = true
        })
        do {
          const page = (await events(db, c.req.param('id'), since, after)) as { id: string | number }[]
          for (const value of page) {
            if (aborted) return
            await s.writeln(JSON.stringify(value))
            after = String(value.id)
          }
          if (page.length < 1000) {
            if (c.req.query('follow') !== 'true') return
            await pause(SECOND_MS)
          }
        } while (!aborted)
      })
    }
    return c.json({ events: await events(db, c.req.param('id'), since) })
  })
  app.post('/v1/webhooks', async (c) =>
    c.json(
      await webhooks.create(c.get('project'), webhookSchema.parse(await c.req.json())),
      StatusCodes.CREATED,
    ),
  )
  app.get('/v1/webhooks', async (c) => c.json({ webhooks: await webhooks.list(c.get('project')) }))
  app.delete('/v1/webhooks/:id', async (c) =>
    c.json(await webhooks.delete(c.get('project'), z.uuid().parse(c.req.param('id')))),
  )
  app.get('/v1/webhooks/:id/deadletters', async (c) =>
    c.json({ deliveries: await webhooks.deadletters(c.get('project'), z.uuid().parse(c.req.param('id'))) }),
  )
  app.post('/v1/templates/:name/versions', async (c) => {
    const name = z
      .string()
      .regex(/^[a-z0-9][a-z0-9-]{0,62}$/)
      .parse(c.req.param('name'))
    return c.json(
      await codeTemplates.register(c.get('project'), name, versionSchema.parse(await c.req.json())),
      StatusCodes.CREATED,
    )
  })
  app.get('/v1/templates/:name/versions', async (c) =>
    c.json({ versions: await codeTemplates.versions(c.get('project'), c.req.param('name')) }),
  )
  app.post('/v1/templates', async (c) => {
    const b = templateSchema.parse(await c.req.json())
    return c.json(await model.template(c.get('project'), b.name, b.spec), StatusCodes.CREATED)
  })
  app.get('/v1/templates', async (c) => c.json({ templates: await model.templates(c.get('project')) }))
  app.notFound((c) =>
    c.json({ error: { code: 'not_found', message: 'No such endpoint' } }, StatusCodes.NOT_FOUND),
  )
  return { app, model, io, agents, webhooks, mounts }
}
