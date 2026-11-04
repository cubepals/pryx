// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public creation and templates map to sandbox intents; private handles and secrets never escape. */
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Make, PublicPort } from './api-schemas.ts'
import { makeSchema } from './api-schemas.ts'
import type { Db } from './database.ts'
import { addresses, DEFAULT_EDGE_PORT_RANGE } from './edge-addresses.ts'
import { shownState } from './edge-routed.ts'
import { complete, fingerprint, operation } from './operations.ts'
import { SandboxError, type Sandboxes, type SandboxSpec } from './sandboxes.ts'
import { jsonb, one, rows } from './sql.ts'

export class PublicModel {
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly domain: string,
    readonly portRange: readonly [number, number] = DEFAULT_EDGE_PORT_RANGE,
  ) {}
  async view(id: string, project: string) {
    const sandbox = await this.boxes.get(id, project)
    const metadata = await one<{ public_ports: PublicPort[] }>(
      this.db,
      sql`SELECT public_ports FROM sandboxes WHERE id = ${id}`,
    )
    const assigned = await addresses(this.db, id, metadata?.public_ports ?? [], this.domain, this.portRange)
    return {
      ...sandbox,
      state: await shownState(this.db, id, sandbox.state),
      endpoints: sandbox.endpoints.map((e) => {
        const p = metadata?.public_ports.find((p) => p.name === e.name)
        return {
          ...e,
          protocol: p?.protocol,
          ...(p?.audience === 'public'
            ? {
                hostname:
                  assigned.find((a) => a.name === p.name)?.hostname ?? `${p.alias ?? id}.${this.domain}`,
                publicPort: assigned.find((a) => a.name === p.name)?.public_port,
              }
            : {}),
        }
      }),
    }
  }
  async create(
    project: string,
    body: Make,
    key?: string,
    options: { id?: string; onlyNode?: string; internal?: boolean } = {},
  ) {
    const op = await operation(this.db, project, 'create', key, body)
    if (op.response) return op.response as Awaited<ReturnType<PublicModel['view']>>
    if ([body.image, body.snapshot, body.template].filter(Boolean).length !== 1)
      throw new SandboxError('invalid_request', 'Choose exactly one of image, snapshot or template')
    let snapshot = body.snapshot
    if (body.template) {
      const copy = await one<{ snapshot: string }>(
        this.db,
        sql`SELECT c.snapshot FROM template_copies c JOIN templates t ON t.id=c.template WHERE (t.id=${body.template} OR t.name=${body.template}) AND t.project=${project} AND c.snapshot IS NOT NULL ORDER BY c.node LIMIT 1`,
      )
      if (!copy) throw new SandboxError('no_room', 'Template has no ready copy')
      snapshot = copy.snapshot
    }
    const source = snapshot ? (await this.boxes.snapshotRecord(snapshot, project)).spec : null
    const spec: SandboxSpec = {
      ...(source ?? {}),
      image: body.image ?? source?.image ?? '',
      resources: body.resources ?? source?.resources ?? { memoryMb: 1024 },
      storage: body.storage ?? source?.storage ?? { mountPath: '/data', sizeGb: 1 },
      env: { ...source?.env, ...body.env },
      secrets: { ...source?.secrets, ...body.secrets },
      labels: body.labels,
      ...(body.entrypoint ? { entrypoint: body.entrypoint } : {}),
      ...(body.restart ? { restart: body.restart } : {}),
      ports: body.ports.map((p) => ({
        name: p.name,
        containerPort: p.port,
        protocol: p.protocol,
        audience: p.audience === 'public' ? ['edge', 'control'] : ['control'],
      })),
      ...(body.ttl ? { ttl: body.ttl } : {}),
      ...(body.idle ? { idle: body.idle } : {}),
    }
    const id = options.id ?? op.id
    if (!options.internal) await addresses(this.db, id, body.ports, this.domain, this.portRange)
    const result = await this.boxes.make(project, spec, {
      id,
      region: body.region,
      start: body.start,
      snapshot,
      publicPorts: body.ports,
      ...options,
    })
    const view = await this.view(result.id, project)
    if (key) await complete(this.db, op.id, view)
    return view
  }
  async template(project: string, name: string, body: Make) {
    if (!body.image || body.snapshot || body.template)
      throw new SandboxError('invalid_request', 'A template starts from an image')
    await this.db.execute(
      sql`INSERT INTO templates(id,project,name,spec) VALUES (${randomUUID()},${project},${name},${jsonb(body)}) ON CONFLICT(project,name) DO NOTHING`,
    )
    const existing = await one<{ spec: Make }>(
      this.db,
      sql`SELECT spec FROM templates WHERE project=${project} AND name=${name}`,
    )
    if (existing && fingerprint(existing.spec) !== fingerprint(body))
      throw new SandboxError('idempotency_key_reused', 'Template name has another spec')
    await this.warmTemplates()
    return (await this.templates(project)).find((t) => t.name === name)
  }
  async templates(project: string) {
    return rows<{ id: string; name: string; copies: unknown[] }>(
      this.db,
      sql`SELECT t.id,t.name,COALESCE(jsonb_agg(jsonb_build_object('node',c.node,'ready',c.snapshot IS NOT NULL,'error',c.error)) FILTER(WHERE c.node IS NOT NULL),'[]') AS copies FROM templates t LEFT JOIN template_copies c ON c.template=t.id WHERE t.project=${project} GROUP BY t.id ORDER BY t.created_at`,
    )
  }
  async warmTemplates() {
    const templates = await rows<{ id: string; project: string; spec: Make }>(
      this.db,
      sql`SELECT * FROM templates`,
    )
    const nodes = await rows<{ id: string; region: string }>(
      this.db,
      sql`SELECT id,region_key AS region FROM fleet_nodes WHERE lifecycle='active' AND duplicate_seen_at IS NULL`,
    )
    for (const t of templates)
      for (const n of nodes) {
        const id = randomUUID()
        await this.db.execute(
          sql`INSERT INTO template_copies(template,node,sandbox) VALUES(${t.id},${n.id},${id}) ON CONFLICT DO NOTHING`,
        )
        const copy = await one<{ sandbox: string; snapshot: string | null }>(
          this.db,
          sql`SELECT * FROM template_copies WHERE template=${t.id} AND node=${n.id}`,
        )
        if (!copy || copy.snapshot) continue
        await this.boxes.exclusive(`template:${t.id}:${n.id}`, async () => {
          const current = await one<{ snapshot: string | null }>(
            this.db,
            sql`SELECT snapshot FROM template_copies WHERE template=${t.id} AND node=${n.id}`,
          )
          if (current?.snapshot) return
          try {
            await this.create(
              t.project,
              makeSchema.parse({ ...t.spec, region: n.region, start: true }),
              undefined,
              { id: copy.sandbox, onlyNode: n.id, internal: true },
            )
            const snap = await this.boxes.snapshot(copy.sandbox, t.project)
            await this.boxes.stop(copy.sandbox, t.project)
            await this.db.execute(
              sql`UPDATE template_copies SET snapshot=${snap.id},error=NULL WHERE template=${t.id} AND node=${n.id}`,
            )
          } catch {
            await this.db.execute(
              sql`UPDATE template_copies SET error='not_ready' WHERE template=${t.id} AND node=${n.id}`,
            )
          }
        })
      }
  }
}
