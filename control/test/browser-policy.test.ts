// SPDX-License-Identifier: FSL-1.1-ALv2
/** The browser wrapper keeps legacy and process launches strict; request environment cannot
 * replace the agent's initial container policy. This exercises the actual template adapter. */
import { expect, test } from 'bun:test'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import contract from '../../openapi/node-writes.openapi.json'
import type { CallOptions } from '../src/node-client.ts'
import { WorkloadApi } from '../src/workload-api.ts'

const sandboxed = createRequire(import.meta.url)('../../templates/browser/sandbox.cjs') as (
  root: string,
) => boolean

test('legacy policy and forged environment retain the browser sandbox', () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-browser-policy-'))
  const original = process.env.PRYX_BROWSER_SANDBOX
  try {
    process.env.PRYX_BROWSER_SANDBOX = '0'
    expect(sandboxed(root)).toBe(true)
    writeFileSync(join(root, 'browser-access.json'), '{invalid')
    expect(sandboxed(root)).toBe(true)
    writeFileSync(join(root, 'browser-access.json'), JSON.stringify({ chromiumSandbox: 'false' }))
    expect(sandboxed(root)).toBe(true)
    writeFileSync(join(root, 'browser-access.json'), JSON.stringify({ token: 'legacy-access' }))
    expect(sandboxed(root)).toBe(true)
    writeFileSync(join(root, 'browser-access.json'), JSON.stringify({ chromiumSandbox: false }))
    expect(sandboxed(root)).toBe(false)
  } finally {
    if (original === undefined) delete process.env.PRYX_BROWSER_SANDBOX
    else process.env.PRYX_BROWSER_SANDBOX = original
  }
})

test('the process wrapper cannot disable Chromium sandboxing', () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-process-browser-policy-'))
  writeFileSync(join(root, 'browser-access.json'), JSON.stringify({ chromiumSandbox: false }))
  const original = process.env.PRYX_PROCESS_POLICY
  try {
    process.env.PRYX_PROCESS_POLICY = ''
    expect(sandboxed(root)).toBe(true)
  } finally {
    if (original === undefined) delete process.env.PRYX_PROCESS_POLICY
    else process.env.PRYX_PROCESS_POLICY = original
  }
})

test('the node receives a browser requirement only after announcing its additive field', async () => {
  let body: Record<string, unknown> = {}
  const api = new WorkloadApi({
    async call<T>(_node: unknown, _method: string, _path: string, options?: CallOptions): Promise<T> {
      body = options?.body as Record<string, unknown>
      return {} as T
    },
  })
  const feature = contract['x-pryx-resources'].nativeBrowserPolicyFeature
  for (const features of [['browser-sandbox'], ['browser-sandbox', feature]]) {
    await api.put(
      { id: 'node', api_address: 'https://node.test', features },
      'box',
      {
        image: 'alpine:browser',
        env: {},
        secrets: {},
        resources: { memoryMb: 512 },
        storage: { mountPath: '/data', sizeGb: 1 },
        ports: [],
        stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
        labels: {},
        require: ['browser-sandbox'],
      },
      1,
    )
    expect(body.requireBrowserSandbox).toBe(features.includes(feature) ? true : undefined)
  }
})
