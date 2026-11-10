// SPDX-License-Identifier: FSL-1.1-ALv2
/** An unsupported policy is refused before any node write, including after feature loss. */
import { expect, test } from 'bun:test'
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
