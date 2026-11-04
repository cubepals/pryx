// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { describe, expect, test } from 'bun:test'
import { DAY_S, HOUR_S } from '@blockly/contracts/durations'
import { deriveHealth, type HealthInput } from './health.ts'

const input = (over: Partial<HealthInput>): HealthInput => ({
  ageSeconds: 1,
  runtimeUp: true,
  reconciled: true,
  listeningSeconds: HOUR_S,
  ...over,
})

describe('deriveHealth', () => {
  test('fresh, with its runtime up and reconciled, is healthy', () => {
    expect(deriveHealth(input({}))).toBe('healthy')
  })

  test('fresh but with Docker down or not yet reconciled is degraded, not dead', () => {
    expect(deriveHealth(input({ runtimeUp: false }))).toBe('degraded')
    expect(deriveHealth(input({ reconciled: false }))).toBe('degraded')
  })

  test('one missed beat is nothing; three are suspect; nine are unavailable', () => {
    expect(deriveHealth(input({ ageSeconds: 10 }))).toBe('healthy')
    expect(deriveHealth(input({ ageSeconds: 16 }))).toBe('suspect')
    expect(deriveHealth(input({ ageSeconds: 45 }))).toBe('suspect')
    expect(deriveHealth(input({ ageSeconds: 46 }))).toBe('unavailable')
    expect(deriveHealth(input({ ageSeconds: DAY_S }))).toBe('unavailable')
  })

  test('never lost: silence alone never produces a verdict', () => {
    const all = [0, 20, 60, HOUR_S, 10 ** 7].map((ageSeconds) => deriveHealth(input({ ageSeconds })))
    expect(all).not.toContain('lost')
  })

  test('a node that never beat is unavailable', () => {
    expect(deriveHealth(input({ ageSeconds: null }))).toBe('unavailable')
  })

  test('a control plane that just started gives every node one suspect window first', () => {
    expect(deriveHealth(input({ ageSeconds: 600, listeningSeconds: 5 }))).toBe('suspect')
    expect(deriveHealth(input({ ageSeconds: 600, listeningSeconds: 50 }))).toBe('unavailable')
  })
})
