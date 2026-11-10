// SPDX-License-Identifier: FSL-1.1-ALv2
/** CPU reservations remain admission inputs; unsupported hard limits never reach compute. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import type { CallOptions } from '../src/node-client.ts'
import { reservedCpu } from '../src/placement.ts'
import { projectUsage } from '../src/project-admission.ts'
import { unitSpec } from '../src/project-model.ts'
import type { RuntimeSpec } from '../src/runtime-model.ts'
import { WorkloadApi } from '../src/workload-api.ts'
import { publicContext } from './public-context.ts'

const spec: RuntimeSpec = {
  image: '',
  isolation: 'process',
  env: {},
  secrets: {},
  resources: { memoryMb: 512, cpuReservationMillis: 250 },
  storage: { mountPath: '/data', sizeGb: 1 },
  ports: [],
  stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
  labels: {},
}

test('unit reservations do not invent a native quota and container quotas stay enforced', () => {
  for (const isolation of ['process', 'wasm', 'container'] as const) {
    const normalized = unitSpec({
      image: '',
      storage: spec.storage,
      resources: spec.resources,
      isolation,
      units: 2,
    })
    expect(normalized.resources.cpuReservationMillis).toBe(500)
    expect(normalized.resources.cpuMillis).toBe(isolation === 'container' ? 500 : undefined)
    expect(reservedCpu(normalized.resources, 250)).toBe(500)
    const explicit = unitSpec({
      image: '',
      storage: spec.storage,
      isolation,
      resources: { memoryMb: 512, cpuMillis: 750 },
    })
    expect(explicit.resources.cpuMillis).toBe(750)
    expect(explicit.units).toBe(3)
  }
  expect(reservedCpu({ memoryMb: 512, cpuMillis: 750, cpuReservationMillis: 250 }, 250)).toBe(750)
  expect(reservedCpu({ memoryMb: 512, cpuMillis: 100, cpuReservationMillis: 500 }, 250)).toBe(500)
})

test('CPU reservations are sent only to nodes announcing the additive field', async () => {
  let body: unknown
  const api = new WorkloadApi({
    async call<T>(_node: unknown, _method: string, _path: string, options?: CallOptions): Promise<T> {
      body = options?.body
      return {} as T
    },
  })
  for (const features of [[], ['cpu-reservation']]) {
    await api.put({ id: 'node', api_address: 'https://node.test', features }, 'box', spec, 1)
    expect((body as { resources: object }).resources).toEqual({
      memoryMb: 512,
      ...(features.length ? { cpuReservationMillis: 250 } : {}),
    })
  }
})

test('public native CPU ceilings and weights fail before any compute call', async () => {
  const c = await publicContext()
  try {
    for (const isolation of ['process', 'wasm']) {
      for (const limit of [{ cpuMillis: 250 }, { cpuWeight: 1024 }]) {
        const before = c.nodes.calls.length
        const response = await c.request('/sandboxes', 'POST', {
          isolation,
          resources: { memoryMb: 512, ...limit },
        })
        expect(response.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
        expect(JSON.stringify(await response.json())).toContain(`resources.${Object.keys(limit)[0]}`)
        expect(c.nodes.calls.length).toBe(before)
      }
    }
  } finally {
    await c.close()
  }
})

test('project accounting counts CPU reservations on records without unit metadata', async () => {
  const c = await publicContext()
  try {
    const made = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', units: 1 })
    expect(made.status).toBe(StatusCodes.CREATED)
    const { id } = (await made.json()) as { id: string }
    await c.db.execute(sql`
      UPDATE sandboxes SET spec=jsonb_set((spec-'units') #- '{resources,cpuMillis}',
        '{resources,cpuReservationMillis}', '1000'::jsonb) WHERE id=${id}
    `)
    expect((await projectUsage(c.db, 'default')).units).toBe(4)
  } finally {
    await c.close()
  }
})

test('workload options and hostname egress require separate feature announcements', async () => {
  let body: unknown
  const api = new WorkloadApi({
    async call<T>(_node: unknown, _method: string, _path: string, options?: CallOptions): Promise<T> {
      body = options?.body
      return {} as T
    },
  })
  for (const features of [
    [],
    ['workload-options'],
    ['egress-hostnames'],
    ['workload-options', 'egress-hostnames'],
  ]) {
    const put = api.put(
      { id: 'node', api_address: 'https://node.test', features },
      'box',
      { ...spec, workdir: '/data/work', user: '65532:65532', egress: { allow: ['example.com'], deny: [] } },
      1,
    )
    if (features.length < 2) await expect(put).rejects.toThrow('Node lacks required features')
    else {
      await put
      const sent = body as Record<string, unknown>
      expect(sent.workdir).toBe('/data/work')
      expect(sent.user).toBe('65532:65532')
      expect(sent.egress).toEqual({ allow: ['example.com'], deny: [] })
    }
  }
})
