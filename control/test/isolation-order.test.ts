// SPDX-License-Identifier: FSL-1.1-ALv2
/** Open tier selection uses healthy registry evidence and scoped guarantees; fixed choices never
 * fall back. These Postgres tests check control decisions, not backend confinement. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { makeSchema } from '../src/api-schemas.ts'
import { PublicModel } from '../src/public-model.ts'
import { publicContext } from './public-context.ts'

const features = [
  'placement-epochs',
  'data-transfer',
  'local-snapshots',
  'export-exclude',
  'agent',
  'files',
  'multi-tier',
  'tier-process',
  'tier-wasm',
  'wasm-packages',
  'tier-process:separate-users',
]

async function tiers(c: Awaited<ReturnType<typeof publicContext>>, offered = features) {
  await c.db.execute(
    sql`UPDATE fleet_nodes SET features=${JSON.stringify(offered)}::jsonb,last_heartbeat_at=now()`,
  )
  await c.boxes.engine.refreshRegistry()
}

test('an unnamed tier chooses the first eligible tier and reports what it received', async () => {
  const c = await publicContext()
  try {
    await tiers(c)
    const made = await c.request('/sandboxes', 'POST', {})
    expect(made.status).toBe(201)
    expect((await made.json()).isolation).toBe('process')
    expect((await (await c.request('/project')).json()).defaults.isolation).toBe('process')
    const ordered = new PublicModel(c.db, c.boxes, 'sandboxes.test', undefined, ['wasm', 'process'])
    const wasm = await ordered.create('default', makeSchema.parse({}))
    expect(wasm.isolation).toBe('wasm')
    const required = await ordered.create('default', makeSchema.parse({ require: ['separate-users'] }))
    expect(required.isolation).toBe('process')
    expect(required.guarantees?.['separate-users']).toBe(true)
    expect(required.guarantees?.['private-network']).toBe(false)
    const before = c.nodes.calls.length
    await expect(ordered.create('default', makeSchema.parse({ require: ['own-kernel'] }))).rejects.toThrow(
      'own-kernel',
    )
    expect(c.nodes.calls).toHaveLength(before)
  } finally {
    await c.close()
  }
})

test('a fixed deployment default refuses an unavailable tier without using another', async () => {
  const c = await publicContext()
  try {
    await tiers(
      c,
      features.filter((feature) => feature !== 'tier-process'),
    )
    const fixed = new PublicModel(c.db, c.boxes, 'sandboxes.test', undefined, 'process')
    const before = c.nodes.calls.length
    await expect(fixed.create('default', makeSchema.parse({}))).rejects.toThrow()
    expect(c.nodes.calls).toHaveLength(before)
    await tiers(c)
    await c.db.execute(sql`UPDATE fleet_nodes SET lifecycle='draining'`)
    await expect(c.model.create('default', makeSchema.parse({}))).rejects.toThrow('No healthy node')
    expect(c.nodes.calls).toHaveLength(before)
  } finally {
    await c.close()
  }
})

test('a legacy template keeps its tier when its omitted isolation is normalized', async () => {
  const c = await publicContext()
  try {
    const body = makeSchema.parse({ image: 'alpine:3.22' })
    const first = await c.model.template('default', 'legacy-tier', body)
    await c.db.execute(sql`UPDATE templates SET spec=spec-'isolation' WHERE name='legacy-tier'`)
    const again = await c.model.template('default', 'legacy-tier', body)
    expect(again?.id).toBe(first?.id)
  } finally {
    await c.close()
  }
})

test('an unnamed private-network request chooses the tier that actually supports its policy', async () => {
  const c = await publicContext()
  try {
    await tiers(c, [
      ...features,
      'tier-container',
      'tier-capabilities:network-policy',
      'network-policy',
      'tier-process:network-policy',
      'tier-process:agent',
      'tier-container:agent',
    ])
    const made = await c.request('/sandboxes', 'POST', { network: 'none' })
    expect(made.status).toBe(StatusCodes.CREATED)
    expect((await made.json()).isolation).toBe('process')
    const before = c.nodes.calls.length
    const refused = await c.request('/sandboxes', 'POST', {
      isolation: 'container',
      image: 'alpine:3.22',
      network: 'none',
    })
    expect(refused.status).toBe(StatusCodes.SERVICE_UNAVAILABLE)
    expect(c.nodes.calls).toHaveLength(before)
  } finally {
    await c.close()
  }
})
