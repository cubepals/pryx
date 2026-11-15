// SPDX-License-Identifier: FSL-1.1-ALv2
/** CPU reservations remain admission inputs; unsupported hard limits never reach compute. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { THRESHOLDS } from '../src/health.ts'
import { VM_CPU_PROBLEM } from '../src/isolation.ts'
import { NodeChoice } from '../src/node-choice.ts'
import type { CallOptions } from '../src/node-client.ts'
import { DEFAULTS, reservedCpu } from '../src/placement.ts'
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

test('a VM ceiling stays whole vCPUs, the most its units pay for', () => {
  for (const [memoryMb, cpuMillis, units, ceiling] of [
    [2560, 1000, 5, 1000],
    [512, 2000, 8, 2000],
    [4096, 1000, 8, 2000],
  ] as const) {
    const normalized = unitSpec({
      image: 'alpine',
      storage: spec.storage,
      isolation: 'vm',
      resources: { memoryMb, cpuMillis },
    })
    expect(normalized.units).toBe(units)
    expect(normalized.resources.cpuMillis).toBe(ceiling)
    expect(normalized.resources.cpuReservationMillis).toBe(units * 250)
  }
  const unnamed = unitSpec({
    image: 'alpine',
    storage: spec.storage,
    isolation: 'vm',
    resources: { memoryMb: 512 },
  })
  expect(unnamed.resources.cpuMillis).toBeUndefined()
})

test('placement asks a VM ceiling for cpu-limit and keeps weights apart', () => {
  const choice = new NodeChoice({ cpuMillisPerGb: 250, thresholds: THRESHOLDS, placement: DEFAULTS })
  const vm = { ...spec, image: 'alpine', isolation: 'vm' as const }
  const ceiling = choice.request({ ...vm, resources: { memoryMb: 512, cpuMillis: 2000 } }, 'eu')
  expect([ceiling.cpuLimits, ceiling.cpuWeight]).toEqual([true, false])
  const weight = choice.request({ ...vm, resources: { memoryMb: 512, cpuWeight: 1024 } }, 'eu')
  expect([weight.cpuLimits, weight.cpuWeight]).toEqual([true, true])
})

test('public VM CPU ceilings must be whole vCPUs and VM weights fail before any compute call', async () => {
  const c = await publicContext()
  try {
    for (const limit of [{ cpuMillis: 1500 }, { cpuMillis: 500 }, { cpuMillis: 0 }, { cpuWeight: 1024 }]) {
      const before = c.nodes.calls.length
      const response = await c.request('/sandboxes', 'POST', {
        isolation: 'vm',
        image: 'alpine:3.22',
        resources: { memoryMb: 512, ...limit },
      })
      expect(response.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
      const body = JSON.stringify(await response.json())
      expect(body).toContain(`resources.${Object.keys(limit)[0]}`)
      if ('cpuMillis' in limit) expect(body).toContain(VM_CPU_PROBLEM)
      expect(c.nodes.calls.length).toBe(before)
    }
    // A whole-vCPU ceiling passes validation and reaches placement, which finds no vm node here.
    const whole = await c.request('/sandboxes', 'POST', {
      isolation: 'vm',
      image: 'alpine:3.22',
      resources: { memoryMb: 512, cpuMillis: 2000 },
    })
    expect(whole.status).not.toBe(StatusCodes.UNPROCESSABLE_ENTITY)
    expect(JSON.stringify(await whole.json())).toContain('no_room')
  } finally {
    await c.close()
  }
})

test('CPU reservations are sent only to nodes announcing the additive field', async () => {
  let body: unknown
  const api = new WorkloadApi({
    async call<T>(_node: unknown, _method: string, _path: string, options?: CallOptions): Promise<T> {
      body = options?.body
      return {} as T
    },
  })
  for (const features of [['process-tier'], ['process-tier', 'cpu-reservation']]) {
    await api.put({ id: 'node', api_address: 'https://node.test', features }, 'box', spec, 1)
    expect((body as { resources: object }).resources).toEqual({
      memoryMb: 512,
      ...(features.includes('cpu-reservation') ? { cpuReservationMillis: 250 } : {}),
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
      {
        ...spec,
        isolation: 'container',
        workdir: '/data/work',
        user: '65532:65532',
        egress: { allow: ['example.com'], deny: [] },
      },
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
