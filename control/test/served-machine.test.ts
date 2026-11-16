// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The machine a joining host must be comes from the ELF headers of the binaries it is handed. */
import { describe, expect, test } from 'bun:test'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { servedMachine } from '../src/served-machine.ts'

/** The first bytes of a little-endian 64-bit ELF executable for `machine` (its e_machine). */
function elf(machine: number) {
  const header = Buffer.alloc(64)
  header.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1])
  header.writeUInt16LE(2, 16)
  header.writeUInt16LE(machine, 18)
  return header
}

describe('served machine', () => {
  const root = mkdtempSync(join(tmpdir(), 'served-machine-'))
  const file = (name: string, bytes: Buffer) => {
    const path = join(root, name)
    writeFileSync(path, bytes)
    return path
  }
  // Linux's include/uapi/linux/elf-em.h, as served-machine.ts reads them.
  const x86 = file('pryxd-x86_64', elf(62))
  const arm = file('pryxd-aarch64', elf(183))
  const agentArm = file('agent-aarch64', elf(183))

  test("is the served pryxd's, not this process's", async () => {
    expect(await servedMachine(x86)).toBe('x86_64')
    expect(await servedMachine(arm)).toBe('aarch64')
    expect(await servedMachine(arm, agentArm)).toBe('aarch64')
    expect(await servedMachine(arm, join(root, 'no-agent'))).toBe('aarch64')
  })
  test('is none when pryxd is missing, not a released machine, or its agent is for another', async () => {
    expect(await servedMachine(join(root, 'missing'))).toBeNull()
    expect(await servedMachine(file('riscv', elf(243)))).toBeNull()
    expect(await servedMachine(file('script', Buffer.from('#!/bin/sh\necho pryxd 1.0.0\n')))).toBeNull()
    expect(await servedMachine(x86, agentArm)).toBeNull()
  })
})
