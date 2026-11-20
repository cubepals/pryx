// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise the baseline collector's summary and distinct creation-wave transport. */
import { expect, test } from 'bun:test'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { baselineSummary, collectBaseline } from '../bench/pryx-baseline.ts'
import { REPETITIONS } from '../bench/tiers/report.ts'

test('retains nearest-rank p50 and p95 and labels empty cohorts', () => {
  expect(baselineSummary({ completed: Array.from({ length: 20 }, (_, i) => 20 - i), refused: [] })).toEqual({
    completed: { n: 20, p50: 10, p95: 19 },
    refused: { n: 0, p50: null, p95: null },
  })
})

test('uses the dedicated SDK only for creation waves', async () => {
  let serial = 0
  let concurrent = 0
  let content: Uint8Array = new Uint8Array()
  const sandbox = {
    state: 'running',
    isolation: 'vm',
    units: 1,
    delete: async () => {},
    run: async () => ({ code: 0, stdout: '', stderr: '' }),
    write: async (_: string, data: Uint8Array) => {
      content = data
    },
    sleep: async () => {},
    wake: async () => {},
    files: {
      stat: async () => ({ size: content.length }),
      readBytes: async () => content,
      delete: async () => {},
    },
  } as unknown as Sandbox
  const result = await collectBaseline(
    {
      sdk: {
        sandbox: async () => {
          serial++
          return sandbox
        },
      },
      concurrentSdk: {
        sandbox: async () => {
          concurrent++
          return sandbox
        },
      },
      memory: async () => 0,
    },
    'vm',
    {},
    () => {},
  )
  expect(serial).toBe(REPETITIONS + 1)
  expect(result.complete).toBe(true)
  expect(result.concurrentCreates.map((wave) => wave.requested)).toEqual([
    1, 1, 1, 1, 1, 10, 10, 10, 10, 10, 50, 50, 50, 50, 50,
  ])
  expect(concurrent).toBe(result.concurrentCreates.reduce((sum, wave) => sum + wave.requested, 0))
})
