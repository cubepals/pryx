// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The gate: what it lets through per server and per address, and what it forgets. */

import { expect, test } from 'bun:test'
import { SECOND_MS } from '../../control/src/durations.ts'
import { addressKey, Gate, type Limits } from '../src/limits.ts'

const LIMITS: Limits = {
  address: { connections: { perSecond: 1, burst: 3 }, greeting: 2, open: 3 },
  server: { connections: { perSecond: 2, burst: 4 }, open: 5 },
}

function clock() {
  let now = 0
  return { now: () => now, pass: (ms: number) => (now += ms) }
}

test('a flood on one server is refused there, and another server still takes connections', () => {
  const gate = new Gate(LIMITS, clock().now)
  const flooded = Array.from({ length: 10 }, () => gate.admit('busy.play.test'))
  expect(flooded.filter(Boolean)).toHaveLength(4)
  expect(gate.admit('quiet.play.test')).not.toBeNull()
})

test('a server takes connections again as its rate refills', () => {
  const time = clock()
  const gate = new Gate(LIMITS, time.now)
  for (let i = 0; i < 4; i++) gate.admit('a.play.test')?.()
  expect(gate.admit('a.play.test')).toBeNull()
  time.pass(SECOND_MS / 2)
  expect(gate.admit('a.play.test')).not.toBeNull()
  expect(gate.admit('a.play.test')).toBeNull()
})

test('a server holds only so many connections open, and a closed one frees its place', () => {
  const time = clock()
  const gate = new Gate(
    { ...LIMITS, server: { connections: { perSecond: 100, burst: 100 }, open: 2 } },
    time.now,
  )
  const first = gate.admit('a.play.test')
  gate.admit('a.play.test')
  expect(gate.admit('a.play.test')).toBeNull()
  first?.()
  first?.()
  expect(gate.admit('a.play.test')).not.toBeNull()
  expect(gate.admit('a.play.test')).toBeNull()
})

test('an address may open only so many connections a second, and only so many still greeting', () => {
  const time = clock()
  const gate = new Gate(LIMITS, time.now)
  const greeting = [gate.greet('203.0.113.7'), gate.greet('203.0.113.7')]
  expect(gate.greet('203.0.113.7')).toBeNull()
  for (const done of greeting) done?.()
  expect(gate.greet('203.0.113.7')).not.toBeNull()
  expect(gate.greet('203.0.113.7')).toBeNull()
  expect(gate.greet('203.0.113.8')).not.toBeNull()
})

test('an IPv6 address counts by its /64, and an IPv4 one written as IPv6 as itself', () => {
  expect(addressKey('2001:db8:0:1::1')).toBe('2001:db8:0:1::/64')
  expect(addressKey('2001:0db8:0000:0001:ffff:0:0:2')).toBe('2001:db8:0:1::/64')
  expect(addressKey('::ffff:203.0.113.7')).toBe('203.0.113.7')
  expect(addressKey('203.0.113.7')).toBe('203.0.113.7')
})

test('a sweep says what was refused, once, and forgets what limits nothing', () => {
  const time = clock()
  const gate = new Gate(LIMITS, time.now)
  for (let i = 0; i < 6; i++) gate.admit('a.play.test')
  expect(gate.sweep()).toEqual(new Map([['server-rate a.play.test', 2]]))
  expect(gate.sweep()).toEqual(new Map())
})

test('an address holds only so many connections let through, across servers, and a closed one frees its place', () => {
  const gate = new Gate(
    { ...LIMITS, server: { connections: { perSecond: 100, burst: 100 }, open: 100 } },
    clock().now,
  )
  const first = gate.admit('a.play.test', '203.0.113.7')
  gate.admit('b.play.test', '203.0.113.7')
  gate.admit('a.play.test', '::ffff:203.0.113.7')
  expect(gate.admit('c.play.test', '203.0.113.7')).toBeNull()
  expect(gate.admit('c.play.test', '203.0.113.8')).not.toBeNull()
  first?.()
  first?.()
  expect(gate.admit('c.play.test', '203.0.113.7')).not.toBeNull()
  expect(gate.admit('c.play.test', '203.0.113.7')).toBeNull()
  expect(gate.sweep()).toEqual(new Map([['address-open 203.0.113.7', 2]]))
})

test('an IPv6 address holds its connections by its /64', () => {
  const gate = new Gate(
    { ...LIMITS, server: { connections: { perSecond: 100, burst: 100 }, open: 100 } },
    clock().now,
  )
  for (const host of ['1', '2', '3']) gate.admit('a.play.test', `2001:db8:0:1::${host}`)
  expect(gate.admit('a.play.test', '2001:db8:0:1::4')).toBeNull()
  expect(gate.admit('a.play.test', '2001:db8:0:2::1')).not.toBeNull()
})

test('a connection given no address is counted against its server alone', () => {
  const gate = new Gate(
    { ...LIMITS, server: { connections: { perSecond: 100, burst: 100 }, open: 100 } },
    clock().now,
  )
  const admitted = Array.from({ length: 6 }, () => gate.admit('a.play.test'))
  expect(admitted.every((release) => release !== null)).toBe(true)
})
