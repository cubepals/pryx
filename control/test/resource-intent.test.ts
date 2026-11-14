// SPDX-License-Identifier: FSL-1.1-ALv2
/** Original public limit intent survives unit rounding and persistence. These real Postgres
 * cases prove placement and node writes; the engine suite proves actual enforcement/refusal. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import contract from '../../openapi/node-writes.openapi.json'
import { UNIT_DEFAULTS } from '../src/project-model.ts'
import type { RuntimeSpec } from '../src/runtime-model.ts'
import { WorkloadApi } from '../src/workload-api.ts'
import { publicContext } from './public-context.ts'

const acceptance = contract['x-pryx-resources'].unenforcedMemoryFeature
const features = [
  'agent',
  'files',
  'placement-epochs',
  'data-transfer',
  'local-snapshots',
  'export-exclude',
  'multi-tier',
  'tier-container',
  'cpu-reservation',
  acceptance,
]
async function offer(c: Awaited<ReturnType<typeof publicContext>>, limits: string[] = []) {
  await c.db.execute(
    sql`UPDATE fleet_nodes SET features=${JSON.stringify([...features, ...limits])}::jsonb,last_heartbeat_at=now()`,
  )
  await c.boxes.engine.refreshRegistry()
}
function lastResources(c: Awaited<ReturnType<typeof publicContext>>) {
  const call = c.nodes.calls.findLast(
    (call) => call.method === 'PUT' && call.path.startsWith('/v1/workloads/'),
  )
  return (call?.body as { resources: Record<string, unknown> }).resources
}

test('omitted public limits persist as defaults and send only announced missing enforcement acceptance', async () => {
  const c = await publicContext()
  try {
    await offer(c)
    const made = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    expect(made.status).toBe(StatusCodes.CREATED)
    const view = (await made.json()) as { id: string }
    expect(view).not.toHaveProperty('resourceDefaults')
    const row = await c.boxes.record(view.id, 'default')
    expect(row.spec.resourceDefaults).toEqual({ memory: true, cpu: true })
    expect(row.spec.resources.memoryMb).toBe(UNIT_DEFAULTS.memoryMb)
    expect(row.spec.resources.cpuMillis).toBe(UNIT_DEFAULTS.cpuMillis)
    expect(lastResources(c)).toEqual({
      memoryMb: UNIT_DEFAULTS.memoryMb,
      cpuReservationMillis: UNIT_DEFAULTS.cpuMillis,
      allowUnenforcedMemory: true,
    })
  } finally {
    await c.close()
  }
})

test('a named memory limit can run without an invented CPU ceiling when memory is enforced', async () => {
  const c = await publicContext()
  try {
    await offer(c, ['tier-container:memory-limit'])
    const made = await c.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      resources: { memoryMb: UNIT_DEFAULTS.memoryMb },
    })
    expect(made.status).toBe(StatusCodes.CREATED)
    const view = (await made.json()) as { id: string }
    expect((await c.boxes.record(view.id, 'default')).spec.resourceDefaults).toEqual({
      memory: false,
      cpu: true,
    })
    expect(lastResources(c)).toMatchObject({
      memoryMb: UNIT_DEFAULTS.memoryMb,
      cpuReservationMillis: UNIT_DEFAULTS.cpuMillis,
    })
    expect(lastResources(c)).not.toHaveProperty('cpuMillis')
    expect(lastResources(c)).not.toHaveProperty('allowUnenforcedMemory')
  } finally {
    await c.close()
  }
})

test('explicit default-valued CPU ceilings and required guarantees still refuse before any node write', async () => {
  const c = await publicContext()
  try {
    await offer(c)
    for (const body of [
      { resources: { memoryMb: UNIT_DEFAULTS.memoryMb, cpuMillis: UNIT_DEFAULTS.cpuMillis } },
      { resources: { memoryMb: UNIT_DEFAULTS.memoryMb } },
      { resources: { memoryMb: UNIT_DEFAULTS.memoryMb, allowUnenforcedMemory: false } },
      { units: 1 },
      { require: ['memory-limit'] },
      { require: ['cpu-limit'] },
    ]) {
      const before = c.nodes.calls.length
      const made = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', ...body })
      expect(made.status).toBe(StatusCodes.SERVICE_UNAVAILABLE)
      expect(c.nodes.calls).toHaveLength(before)
    }
  } finally {
    await c.close()
  }
})

test('snapshot forks retain recorded intent and legacy snapshots never guess defaults from their numbers', async () => {
  const c = await publicContext()
  try {
    await offer(c)
    const first = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    expect(first.status).toBe(StatusCodes.CREATED)
    const original = (await first.json()) as { id: string }
    const taken = await c.request(`/sandboxes/${original.id}/snapshots`, 'POST')
    expect(taken.status).toBe(StatusCodes.CREATED)
    const snapshot = (await taken.json()) as { id: string }
    const fork = await c.request(`/snapshots/${snapshot.id}/sandboxes`, 'POST', {})
    expect(fork.status).toBe(StatusCodes.CREATED)
    const copy = (await fork.json()) as { id: string }
    expect((await c.boxes.record(copy.id, 'default')).spec.resourceDefaults).toEqual({
      memory: true,
      cpu: true,
    })
    expect(lastResources(c).allowUnenforcedMemory).toBe(true)
    await c.db.execute(sql`UPDATE sandbox_snapshots SET spec=spec-'resourceDefaults' WHERE id=${snapshot.id}`)
    await offer(c, ['tier-container:memory-limit', 'tier-container:cpu-limit'])
    const legacy = await c.request(`/snapshots/${snapshot.id}/sandboxes`, 'POST', {})
    expect(legacy.status).toBe(StatusCodes.CREATED)
    const old = (await legacy.json()) as { id: string }
    expect((await c.boxes.record(old.id, 'default')).spec.resourceDefaults).toBeUndefined()
    expect(lastResources(c)).toMatchObject({
      memoryMb: UNIT_DEFAULTS.memoryMb,
      cpuMillis: UNIT_DEFAULTS.cpuMillis,
    })
    expect(lastResources(c)).not.toHaveProperty('allowUnenforcedMemory')
  } finally {
    await c.close()
  }
})

test('legacy records and nodes retain exact resource bytes and capable nodes retain generated ceilings', async () => {
  const bodies: Array<{ resources: RuntimeSpec['resources'] }> = []
  const api = new WorkloadApi({
    async call<T>(...args: unknown[]): Promise<T> {
      bodies.push((args[3] as { body: { resources: RuntimeSpec['resources'] } }).body)
      return {} as T
    },
  })
  const spec: RuntimeSpec = {
    image: 'alpine:3.22',
    isolation: 'container',
    env: {},
    secrets: {},
    resources: { memoryMb: UNIT_DEFAULTS.memoryMb, cpuMillis: UNIT_DEFAULTS.cpuMillis },
    storage: { mountPath: '/data', sizeGb: 1 },
    ports: [],
    stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
    labels: {},
  }
  const node = { id: 'node', api_address: 'https://node.test', features: [] as string[] }
  const defaults = { memory: true, cpu: true }
  await api.put(node, 'legacy-node', { ...spec, resourceDefaults: defaults }, 1)
  await api.put({ ...node, features: [acceptance] }, 'legacy-record', spec, 1)
  await api.put(
    {
      ...node,
      features: [...features, 'tier-container:memory-limit', 'tier-container:cpu-limit'],
    },
    'capable',
    { ...spec, resourceDefaults: defaults },
    1,
  )
  for (const body of bodies) expect(JSON.stringify(body.resources)).toBe(JSON.stringify(spec.resources))
})
