// SPDX-License-Identifier: FSL-1.1-ALv2
/** Required enforcement survives normalization and refuses placement without a compute call. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { GUARANTEES, guarantees } from '../src/guarantees.ts'
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
