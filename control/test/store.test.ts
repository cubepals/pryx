// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** store test behavior and its local regression coverage. */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createHash, randomBytes } from 'node:crypto'
import { FleetStore } from '../src/archive-transport.ts'
import { MemoryStore } from './memory-store.ts'

const MIB = 1024 ** 2
const sha256 = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')

// The store copy, from a fleet copy to an application's archive, against a store in this process:
// in one PUT, or read by its ranges into parts when it is larger than one PUT carries.
describe('FleetStore.copyTo', () => {
  const memory = new MemoryStore()
  const fleet = new FleetStore(memory, 'test')
  // Bytes that don't compress, 20 MiB and a little: three parts of 8 MiB, the last shorter.
  const data = randomBytes(20 * MIB + 777)
  const source = fleet.objectKey('server', 'copy')

  beforeAll(async () => {
    await memory.start()
    memory.objects.set(source, data)
  })

  afterAll(() => memory.close())

  const copy = async (key: string) => fleet.copyTo(source, await memory.archiveTarget(key))

  test('one PUT carries a copy no larger than it, hashed on the way', async () => {
    memory.maxPutBytes = data.length
    expect(await copy('archives/whole')).toEqual({
      sizeBytes: data.length,
      sha256: sha256(data),
    })
    expect(memory.objects.get('archives/whole')?.equals(data)).toBe(true)
    expect(memory.uploads.size).toBe(0)
  })

  test('a larger one is read by its ranges into parts, in order, so its hash is of the whole', async () => {
    memory.maxPutBytes = 4 * MIB
    const completed = memory.completed
    expect(await copy('archives/parts')).toEqual({
      sizeBytes: data.length,
      sha256: sha256(data),
    })
    expect(memory.objects.get('archives/parts')?.equals(data)).toBe(true)
    expect(memory.completed).toBe(completed + 1)
    expect(memory.uploads.size).toBe(0)
  })

  test('a part the store fails is sent again, and hashed once', async () => {
    memory.maxPutBytes = 4 * MIB
    // The first part is answered 500, then 503, and goes the third time.
    memory.refuse.push(500, 503)
    expect(await copy('archives/again')).toEqual({
      sizeBytes: data.length,
      sha256: sha256(data),
    })
    expect(memory.refuse).toEqual([])
    expect(memory.objects.get('archives/again')?.equals(data)).toBe(true)
  })

  test('a part refused for good drops the upload in parts and fails the copy', async () => {
    memory.maxPutBytes = 4 * MIB
    const aborted = memory.aborted
    memory.refuse.push(403)
    await expect(copy('archives/refused')).rejects.toThrow('HTTP 403')
    expect(memory.refuse).toEqual([])
    expect(memory.aborted).toBe(aborted + 1)
    expect(memory.objects.has('archives/refused')).toBe(false)
    expect(memory.uploads.size).toBe(0)
  })

  test('three failures of one part fail the copy', async () => {
    memory.maxPutBytes = 4 * MIB
    memory.refuse.push(500, 500, 500)
    await expect(copy('archives/busy')).rejects.toThrow('HTTP 500')
    expect(memory.refuse).toEqual([])
    expect(memory.objects.has('archives/busy')).toBe(false)
  })

  test('nothing at the key is a plain failure, and nothing is begun', async () => {
    await expect(
      fleet.copyTo('fleet/test/none/none.tar.gz', await memory.archiveTarget('archives/none')),
    ).rejects.toThrow('The store holds nothing at fleet/test/none/none.tar.gz')
    expect(memory.uploads.size).toBe(0)
  })
})
