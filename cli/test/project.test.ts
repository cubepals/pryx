// SPDX-License-Identifier: FSL-1.1-ALv2
/** CLI project flags preserve units, duration semantics and explicit inheritance resets. */
import { expect, test } from 'bun:test'
import { DAY_MS, HOUR_S } from '../../control/src/durations.ts'
import { projectOptions } from '../src/project.ts'

test('project limits parse numbers, durations, image prefixes and expiry', () => {
  const before = Date.now()
  const options = projectOptions([
    'create',
    '--max-units',
    '5',
    '--max-running',
    '3',
    '--max-ttl',
    '2h',
    '--expires-in',
    '24h',
    '--allowed-images',
    'alpine:,node:',
    '--key',
  ])
  expect(options).toMatchObject({
    maxUnits: 5,
    maxRunning: 3,
    maxTtl: 2 * HOUR_S,
    allowedImages: ['alpine:', 'node:'],
    key: true,
  })
  expect(Date.parse(options.expiresAt as string)).toBeGreaterThanOrEqual(before + DAY_MS)
  expect(Date.parse(options.expiresAt as string)).toBeLessThanOrEqual(Date.now() + DAY_MS)
  expect(projectOptions(['set', 'p', '--max-units', 'unlimited'])).toEqual({ maxUnits: null })
})
