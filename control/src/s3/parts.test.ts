// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { describe, expect, test } from 'bun:test'
import { MAX_PUT_BYTES, partPlan } from './parts.ts'

const MIB = 1024 ** 2
const GIB = 1024 ** 3

describe('partPlan', () => {
  test('one PUT carries no more than R2 takes in one', () => {
    expect(MAX_PUT_BYTES).toBe(5 * GIB - 5 * MIB)
  })

  test.each([MAX_PUT_BYTES + 1, 6 * GIB, 50 * GIB, 300 * GIB, 30 * 1024 * GIB])(
    'an archive of %d bytes fits its parts, whole MiB each, within what S3 and R2 take',
    (size) => {
      const { partSize, count } = partPlan(size)
      expect(partSize % MIB).toBe(0)
      expect(partSize).toBeGreaterThanOrEqual(8 * MIB)
      expect(partSize).toBeLessThanOrEqual(4 * GIB)
      expect(count).toBeLessThanOrEqual(10_000)
      // Room for an archive packed again a little larger.
      expect(partSize * count).toBeGreaterThan(size * 1.03)
    },
  )

  test('about as many parts as asked, unless the parts would be larger than any store takes', () => {
    expect(partPlan(6 * GIB).count).toBeLessThanOrEqual(65)
    expect(partPlan(6 * GIB, 8).count).toBeLessThanOrEqual(9)
    expect(partPlan(1024 * GIB, 8)).toEqual({ partSize: 4 * GIB, count: 265 })
  })

  test('a small object is still parts of 8 MiB, the least the plan makes', () => {
    expect(partPlan(1)).toEqual({ partSize: 8 * MIB, count: 1 })
    expect(partPlan(20 * MIB, 64)).toEqual({ partSize: 8 * MIB, count: 3 })
  })

  test('more than 10,000 parts of the largest size hold is refused', () => {
    expect(() => partPlan(40_000 * GIB)).toThrow('more than 10000 parts hold')
  })
})
