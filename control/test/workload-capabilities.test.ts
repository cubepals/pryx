// SPDX-License-Identifier: FSL-1.1-ALv2
/** An unsupported policy is refused before any node write, including after feature loss. */
import { expect, test } from 'bun:test'
import contract from '../../openapi/node-writes.openapi.json'
import { makeSchema } from '../src/api-schemas.ts'
import type { RuntimeSpec } from '../src/runtime-model.ts'
import { WorkloadApi } from '../src/workload-api.ts'

const spec: RuntimeSpec = {
  image: 'alpine:3.22',
  env: {},
  secrets: {},
  resources: { memoryMb: 512 },
  storage: { mountPath: '/data', sizeGb: 1 },
  ports: [],
  stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
  labels: {},
}

test('policy fields cannot disappear on a node lacking their feature', async () => {
  let calls = 0
  const api = new WorkloadApi({
    async call<T>(): Promise<T> {
      calls++
      return {} as T
    },
  })
  const node = { id: 'node', api_address: 'https://node.test', features: [] as string[] }
  const policies: [Partial<RuntimeSpec>, string][] = [
    [{ workdir: '/data' }, 'workload-options'],
    [{ user: '65532' }, 'workload-options'],
    [{ egress: { allow: ['example.com'] } }, 'egress-hostnames'],
    [{ network: 'none' as const }, 'network-policy'],
    [{ packages: ['python'] }, 'wasm-packages'],
    [{ requiredTools: ['desktop'] }, contract['x-pryx-native-tools'].desktopFeature],
    [{ requiredTools: ['ssh'] }, contract['x-pryx-native-tools'].sshFeature],
    [
      { resources: { memoryMb: 256, allowUnenforcedMemory: true } },
      contract['x-pryx-resources'].unenforcedMemoryFeature,
    ],
  ]
  for (const [field, feature] of policies) {
    const before = calls
    await expect(api.put(node, 'box', { ...spec, ...field }, 1)).rejects.toThrow(feature)
    expect(calls).toBe(before)
    await api.put({ ...node, features: [feature] }, 'box', { ...spec, ...field }, 1)
    expect(calls).toBe(before + 1)
  }
  await api.put(node, 'box', spec, 1)
  await api.put(node, 'box', { ...spec, network: 'internet' }, 1)
})

test('tier selection reaches only announcing nodes and cannot inherit another tier guarantees', async () => {
  const bodies: Record<string, unknown>[] = []
  const api = new WorkloadApi({
    async call<T>(...args: unknown[]): Promise<T> {
      bodies.push((args[3] as { body: Record<string, unknown> }).body)
      return {} as T
    },
  })
  const node = { id: 'node', api_address: 'https://node.test', features: [] as string[] }
  await api.put(node, 'box', { ...spec, isolation: 'container' }, 1)
  expect(bodies[0]).not.toHaveProperty('isolation')
  await api.put(
    { ...node, features: ['process-tier'] },
    'box',
    { ...spec, isolation: 'process', image: '' },
    1,
  )
  expect(bodies[1]).not.toHaveProperty('isolation')
  const features = ['multi-tier', 'tier-container', 'tier-process', 'cpu-limit', 'tier-container:cpu-limit']
  await api.put({ ...node, features }, 'box', { ...spec, isolation: 'process', image: '' }, 1)
  expect(bodies[2]?.isolation).toBe('process')
  await expect(
    api.put(
      { ...node, features },
      'box',
      {
        ...spec,
        isolation: 'process',
        image: '',
        require: ['cpu-limit'],
      },
      1,
    ),
  ).rejects.toThrow('cpu-limit')
  expect(bodies).toHaveLength(3)
})

test('memory acceptance is feature gated for both booleans and null remains absent on legacy nodes', async () => {
  const bodies: Array<{ resources: Record<string, unknown> }> = []
  const api = new WorkloadApi({
    async call<T>(...args: unknown[]): Promise<T> {
      bodies.push((args[3] as { body: { resources: Record<string, unknown> } }).body)
      return {} as T
    },
  })
  const node = { id: 'node', api_address: 'https://node.test', features: [] as string[] }
  const feature = contract['x-pryx-resources'].unenforcedMemoryFeature
  for (const acceptance of [false, true]) {
    const request = { ...spec, resources: { ...spec.resources, allowUnenforcedMemory: acceptance } }
    const before = bodies.length
    await expect(api.put(node, 'box', request, 1)).rejects.toThrow(feature)
    expect(bodies).toHaveLength(before)
    await api.put({ ...node, features: [feature] }, 'box', request, 1)
    expect(bodies.at(-1)?.resources.allowUnenforcedMemory).toBe(acceptance)
  }
  await api.put(node, 'box', { ...spec, resources: { ...spec.resources, allowUnenforcedMemory: null } }, 1)
  expect(bodies.at(-1)?.resources).not.toHaveProperty('allowUnenforcedMemory')
})

test('scoped network policy is serialized for its tier and refused before an unchanged-spec wake elsewhere', async () => {
  const bodies: Record<string, unknown>[] = []
  const api = new WorkloadApi({
    async call<T>(...args: unknown[]): Promise<T> {
      bodies.push((args[3] as { body: Record<string, unknown> }).body)
      return {} as T
    },
  })
  const feature = contract['x-pryx-network'].policyFeature
  const marker = `${contract['x-pryx-tiers'].capabilityScopePrefix}${feature}`
  const features = ['multi-tier', 'tier-container', 'tier-process', marker, `tier-process:${feature}`]
  const node = { id: 'node', api_address: 'https://node.test', features }
  const process = { ...spec, image: '', isolation: 'process' as const, network: 'none' as const }
  await api.put(node, 'process', process, 1)
  expect(bodies[0]?.network).toBe('none')
  await expect(
    api.put(node, 'container', { ...spec, isolation: 'container', network: 'none' }, 1),
  ).rejects.toThrow(feature)
  await expect(
    api.put(
      { ...node, features: features.filter((value) => !value.startsWith('tier-process:')) },
      'process',
      process,
      1,
      { wake: true },
    ),
  ).rejects.toThrow(feature)
  expect(bodies).toHaveLength(1)
})

test('native tool requirements stay in control and refuse feature loss before an unchanged wake', async () => {
  const bodies: Record<string, unknown>[] = []
  const api = new WorkloadApi({
    async call<T>(...args: unknown[]): Promise<T> {
      bodies.push((args[3] as { body: Record<string, unknown> }).body)
      return {} as T
    },
  })
  const tools = contract['x-pryx-native-tools']
  const features = [
    'multi-tier',
    'tier-process',
    'tier-container',
    ...[tools.desktopFeature, tools.sshFeature].flatMap((feature) => [
      `${contract['x-pryx-tiers'].capabilityScopePrefix}${feature}`,
      `tier-process:${feature}`,
    ]),
  ]
  const node = { id: 'node', api_address: 'https://node.test', features }
  const request: RuntimeSpec = { ...spec, image: '', isolation: 'process', requiredTools: ['desktop', 'ssh'] }
  await api.put(node, 'native', request, 1)
  expect(bodies[0]).not.toHaveProperty('requiredTools')
  await expect(
    api.put(
      { ...node, features: features.filter((feature) => feature !== `tier-process:${tools.sshFeature}`) },
      'native',
      request,
      1,
      { wake: true },
    ),
  ).rejects.toThrow(tools.sshFeature)
  await expect(api.put(node, 'container', { ...request, isolation: 'container' }, 1)).rejects.toThrow(
    tools.desktopFeature,
  )
  await expect(api.put(node, 'native', { ...request, network: 'none' }, 1)).rejects.toThrow('network-none')
  expect(bodies).toHaveLength(1)
})

test('tool requirements reject unknown names and network-none before placement', () => {
  expect(makeSchema.parse({ isolation: 'process', requiredTools: ['desktop', 'ssh'] })).toHaveProperty(
    'requiredTools',
    ['desktop', 'ssh'],
  )
  expect(() => makeSchema.parse({ requiredTools: ['unannounced'] })).toThrow()
  expect(() => makeSchema.parse({ network: 'none', requiredTools: ['ssh'] })).toThrow('network-none')
})
