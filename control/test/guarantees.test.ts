// SPDX-License-Identifier: FSL-1.1-ALv2
/** Required enforcement survives normalization and refuses placement without a compute call. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import contract from '../../openapi/node-writes.openapi.json'
import { GUARANTEES, guarantees, workloadGuarantees } from '../src/guarantees.ts'
import { runtimeSpec } from '../src/sandboxes.ts'
import { publicContext } from './public-context.ts'

test('unannounced guarantees are false and requested names survive runtime translation', () => {
  const result = guarantees(['memory-limit', 'unrelated'])
  for (const name of GUARANTEES) expect(result[name]).toBe(name === 'memory-limit')
  expect(
    runtimeSpec({
      image: 'alpine:3.22',
      resources: { memoryMb: 512 },
      storage: { mountPath: '/data', sizeGb: 1 },
      require: ['own-kernel'],
    }).require,
  ).toEqual(['own-kernel'])
  expect(
    workloadGuarantees([contract['x-pryx-resources'].observedGuaranteesFeature, 'memory-limit'], 'process'),
  ).toEqual(guarantees(['memory-limit'], 'process'))
})

test('public create names a missing guarantee and leaves compute untouched', async () => {
  const c = await publicContext()
  try {
    const before = c.nodes.calls.length
    const response = await c.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      require: ['own-kernel'],
    })
    expect(response.status).toBe(StatusCodes.SERVICE_UNAVAILABLE)
    const body = await response.json()
    expect(body.error.code).toBe('no_room')
    expect(body.error.message).toContain('own-kernel')
    expect(c.nodes.calls.length).toBe(before)
    const ordinary = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    expect(ordinary.status).toBe(StatusCodes.CREATED)
    const made = await ordinary.json()
    expect(made.guarantees).toEqual(guarantees([]))
  } finally {
    await c.close()
  }
})

test('an unavailable named tier is refused by name without contacting compute', async () => {
  const c = await publicContext()
  try {
    const before = c.nodes.calls.length
    const response = await c.request('/sandboxes', 'POST', { isolation: 'wasm' })
    expect(response.status).toBe(StatusCodes.SERVICE_UNAVAILABLE)
    const body = await response.json()
    expect(body.error.code).toBe('no_room')
    expect(body.error.message).toContain('wasm')
    expect(c.nodes.calls.length).toBe(before)
  } finally {
    await c.close()
  }
})

test('public rootless limits follow the current copy observation across controller gains', async () => {
  const c = await publicContext()
  try {
    const created = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    expect(created.status).toBe(StatusCodes.CREATED)
    const { id } = (await created.json()) as { id: string }
    await c.report()
    const feature = contract['x-pryx-resources'].observedGuaranteesFeature
    const offered = [
      'multi-tier',
      'tier-container',
      'tier-container:memory-limit',
      'tier-container:cpu-limit',
      'tier-container:private-pids',
    ]
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify([...offered, feature])}::jsonb`)
    const view = async () => {
      const response = await c.request(`/sandboxes/${id}`)
      expect(response.status).toBe(StatusCodes.OK)
      return (await response.json()).guarantees
    }
    const limits = async (memory: boolean, cpu: boolean) => {
      await c.db.execute(sql`UPDATE fleet_observations SET report=jsonb_set(report,'{guarantees}',
        ${JSON.stringify({ 'memory-limit': memory, 'cpu-limit': cpu })}::jsonb) WHERE workload=${id}`)
    }
    await limits(false, false)
    expect(await view()).toMatchObject({ 'memory-limit': false, 'cpu-limit': false, 'private-pids': true })
    await limits(true, false)
    expect(await view()).toMatchObject({ 'memory-limit': true, 'cpu-limit': false })
    await limits(true, true)
    expect(await view()).toMatchObject({ 'memory-limit': true, 'cpu-limit': true })
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify([feature])}::jsonb`)
    expect(await view()).toMatchObject({ 'memory-limit': false, 'cpu-limit': false })
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify([...offered, feature])}::jsonb`)
    await c.db.execute(sql`UPDATE fleet_observations SET epoch=epoch+1 WHERE workload=${id}`)
    expect(await view()).toMatchObject({ 'memory-limit': false, 'cpu-limit': false })
    await c.db.execute(
      sql`UPDATE fleet_observations SET epoch=epoch-1,report=report-'guarantees' WHERE workload=${id}`,
    )
    expect(await view()).toMatchObject({ 'memory-limit': false, 'cpu-limit': false })
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify(offered)}::jsonb`)
    expect(await view()).toMatchObject({ 'memory-limit': true, 'cpu-limit': true })
  } finally {
    await c.close()
  }
})

test('browser sandbox reporting follows the current copy and retains other legacy guarantees', async () => {
  const c = await publicContext()
  try {
    const created = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    const { id } = (await created.json()) as { id: string }
    await c.report()
    const feature = contract['x-pryx-resources'].nativeBrowserPolicyFeature
    const offered = ['browser-sandbox', 'memory-limit', 'private-network']
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify([...offered, feature])}::jsonb`)
    const view = async () => (await (await c.request(`/sandboxes/${id}`)).json()).guarantees
    const observed = async (on: boolean) =>
      c.db.execute(sql`UPDATE fleet_observations SET report=jsonb_set(report,'{guarantees}',
        ${JSON.stringify({ 'browser-sandbox': on })}::jsonb) WHERE workload=${id}`)
    await observed(false)
    expect(await view()).toMatchObject({
      'browser-sandbox': false,
      'memory-limit': true,
      'private-network': true,
    })
    await observed(true)
    expect((await view())['browser-sandbox']).toBe(true)
    await c.db.execute(sql`UPDATE fleet_observations SET epoch=epoch+1 WHERE workload=${id}`)
    expect((await view())['browser-sandbox']).toBe(false)
    await c.db.execute(
      sql`UPDATE fleet_observations SET epoch=epoch-1,report=report-'guarantees' WHERE workload=${id}`,
    )
    expect((await view())['browser-sandbox']).toBe(false)
    await c.db.execute(sql`UPDATE fleet_nodes SET features=${JSON.stringify(offered)}::jsonb`)
    expect((await view())['browser-sandbox']).toBe(true)
  } finally {
    await c.close()
  }
})
