// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** handle test behavior and its local regression coverage. */

import { describe, expect, test } from 'bun:test'
import { decodeHandle, decodeSnapshot, encodeHandle, encodeSnapshot, type FleetRef } from '../src/handle.ts'

const ref: FleetRef = {
  deployment: 'test',
  key: '5b0f3c1e-0000-4000-8000-000000000001',
  node: 'b1c2d3e4-0000-4000-8000-000000000002',
  nodeName: 'node-a',
  epoch: 3,
  region: 'eu',
  edgeHost: '10.0.0.10',
  controlHost: '10.0.0.10',
  ports: { game: 30001, rcon: 30002 },
}

describe('fleet handles', () => {
  test('round-trip, and change when the epoch does', () => {
    const handle = encodeHandle(ref)
    expect(handle.startsWith('fleet:v1:')).toBe(true)
    expect(decodeHandle(handle)).toEqual(ref)
    expect(encodeHandle({ ...ref, epoch: 4 })).not.toBe(handle)
  })

  test("refuse another runtime's handle, and a malformed one", () => {
    expect(() => decodeHandle('docker:v1:e30')).toThrow('not issued by the fleet runtime')
    expect(() => decodeHandle('fleet:v1:bm90IGpzb24')).toThrow('Malformed')
    expect(() => decodeHandle(`fleet:v1:${Buffer.from('{"key":1}').toString('base64url')}`)).toThrow(
      'Malformed',
    )
  })

  test('a resting server’s handle names no node', () => {
    const resting = encodeHandle({
      ...ref,
      node: null,
      nodeName: null,
      edgeHost: null,
      controlHost: null,
    })
    expect(decodeHandle(resting).node).toBeNull()
  })

  test('snapshots name the copy and the epoch it was taken at', () => {
    const snap = {
      deployment: 'test',
      key: ref.key,
      archive: '9d6b1f3a-0000-4000-8000-000000000003',
      epoch: 3,
    }
    const handle = encodeSnapshot(snap)
    expect(handle.startsWith('fleet-snap:v1:')).toBe(true)
    expect(decodeSnapshot(handle)).toEqual(snap)
    expect(() => decodeSnapshot(encodeHandle(ref))).toThrow('not issued by the fleet runtime')
  })
})
