// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Generated socket histories check independent connection counts, duplicate close events and rate boundaries. */

import { expect, test } from 'bun:test'
import fc from 'fast-check'
import { SECOND_MS } from '../../control/src/durations.ts'
import { addressKey, Bucket, Gate } from '../src/limits.ts'

const properties = { seed: 0xc011ec7, numRuns: 150, includeErrorInReport: true }

test('admission follows the live connections, even when close events repeat or arrive out of order', () => {
  const event = fc.record({
    close: fc.boolean(),
    server: fc.integer({ min: 0, max: 2 }),
    address: fc.integer({ min: 0, max: 2 }),
    ticket: fc.nat(),
  })
  fc.assert(
    fc.property(
      fc.integer({ min: 1, max: 5 }),
      fc.integer({ min: 1, max: 5 }),
      fc.array(event, { minLength: 1, maxLength: 100 }),
      (serverLimit, addressLimit, events) => {
        const connections = { perSecond: 1000, burst: 1000 }
        const gate = new Gate(
          {
            address: { connections, greeting: 10, open: addressLimit },
            server: { connections, open: serverLimit },
          },
          () => 0,
        )
        const tickets: { server: number; address: number; active: boolean; release: () => void }[] = []
        for (const event of events) {
          if (event.close) {
            const ticket = tickets[event.ticket % tickets.length]
            if (ticket !== undefined) {
              ticket.release()
              ticket.active = false
            }
            continue
          }
          const live = tickets.filter((ticket) => ticket.active)
          const allowed =
            live.filter((ticket) => ticket.server === event.server).length < serverLimit &&
            live.filter((ticket) => ticket.address === event.address).length < addressLimit
          const release = gate.admit(`server-${event.server}`, `203.0.113.${event.address}`)
          expect(release !== null).toBe(allowed)
          if (release !== null) tickets.push({ ...event, active: true, release })
        }
        for (const ticket of tickets) ticket.release()
        expect(gate.admit('server-0', '203.0.113.0')).not.toBeNull()
      },
    ),
    properties,
  )
})

test('a bucket admits exactly its burst and refills only at the next token boundary', () => {
  fc.assert(
    fc.property(fc.integer({ min: 1, max: 30 }), fc.integer({ min: 0, max: 5 }), (burst, exponent) => {
      const perSecond = 2 ** exponent
      const bucket = new Bucket({ burst, perSecond }, 0)
      for (let used = 0; used < burst; used++) expect(bucket.take(0)).toBe(true)
      expect(bucket.take(0)).toBe(false)
      expect(bucket.take(SECOND_MS / perSecond - 1)).toBe(false)
      expect(bucket.take(SECOND_MS / perSecond)).toBe(true)
      expect(bucket.take(SECOND_MS / perSecond)).toBe(false)
      expect(bucket.full((burst + 1) * SECOND_MS)).toBe(true)
      for (let used = 0; used < burst; used++) expect(bucket.take((burst + 1) * SECOND_MS)).toBe(true)
      expect(bucket.take((burst + 1) * SECOND_MS)).toBe(false)
    }),
    properties,
  )
})

test('IPv6 spelling and host bits cannot evade the same network connection limit', () => {
  const group = fc.integer({ min: 0, max: 0xffff })
  fc.assert(
    fc.property(fc.tuple(group, group, group, group), group, group, (network, first, second) => {
      const prefix = network.map((part) => part.toString(16)).join(':')
      const padded = network.map((part) => part.toString(16).padStart(4, '0').toUpperCase()).join(':')
      expect(addressKey(`${prefix}::${first.toString(16)}`)).toBe(`${prefix}::/64`)
      expect(addressKey(`${padded}:0000:0000:0000:${second.toString(16)}`)).toBe(`${prefix}::/64`)
    }),
    properties,
  )
})

test('leading and trailing IPv6 compression preserves the network, and mapped IPv4 keeps every octet', () => {
  for (const [address, network] of [
    ['::', '0:0:0:0::/64'],
    ['::1', '0:0:0:0::/64'],
    ['2001:db8::', '2001:db8:0:0::/64'],
    ['2001:db8::ffff:0:0:1', '2001:db8:0:0::/64'],
    ['::ffff:198.51.100.25', '198.51.100.25'],
    ['::FFFF:192.168.255.254', '192.168.255.254'],
  ] as const) {
    expect(addressKey(address)).toBe(network)
  }
})

test('refusal telemetry distinguishes greeting, opening rate and server capacity and is drained once', () => {
  const gate = new Gate(
    {
      address: { connections: { perSecond: 1, burst: 1 }, greeting: 1, open: 2 },
      server: { connections: { perSecond: 10, burst: 10 }, open: 1 },
    },
    () => 0,
  )
  const greeted = gate.greet('203.0.113.1')
  expect(greeted).not.toBeNull()
  expect(gate.greet('203.0.113.1')).toBeNull()
  greeted?.()
  expect(gate.greet('203.0.113.1')).toBeNull()
  expect(gate.admit('busy')).not.toBeNull()
  expect(gate.admit('busy')).toBeNull()
  expect(gate.sweep()).toEqual(
    new Map([
      ['address-greeting 203.0.113.1', 1],
      ['address-rate 203.0.113.1', 1],
      ['server-open busy', 1],
    ]),
  )
  expect(gate.sweep()).toEqual(new Map())
})

test('a maintenance sweep never frees live sockets even when their rate bucket has refilled', () => {
  let now = 0
  const gate = new Gate(
    {
      address: { connections: { perSecond: 1, burst: 1 }, greeting: 1, open: 1 },
      server: { connections: { perSecond: 1, burst: 1 }, open: 1 },
    },
    () => now,
  )
  const greeting = gate.greet('203.0.113.1')
  const admitted = gate.admit('busy')
  expect(greeting).not.toBeNull()
  expect(admitted).not.toBeNull()
  now += SECOND_MS
  gate.sweep()
  expect(gate.greet('203.0.113.1')).toBeNull()
  expect(gate.admit('busy')).toBeNull()
  greeting?.()
  admitted?.()
  expect(gate.greet('203.0.113.1')).not.toBeNull()
  expect(gate.admit('busy')).not.toBeNull()
})
