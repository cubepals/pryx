// SPDX-License-Identifier: FSL-1.1-ALv2
import { expect, test } from 'bun:test'
import { baselineSummary } from '../bench/pryx-baseline.ts'

test('retains nearest-rank p50 and p95 and labels empty cohorts', () => {
  expect(baselineSummary({ completed: Array.from({ length: 20 }, (_, i) => 20 - i), refused: [] })).toEqual({
    completed: { n: 20, p50: 10, p95: 19 },
    refused: { n: 0, p50: null, p95: null },
  })
})
