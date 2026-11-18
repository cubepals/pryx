// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The machine a joining host has to be to run what this control plane hands it at
 * /fleet/v1/blocklyd and /fleet/v1/pryx-agent (join-routes.ts), read from those binaries' own ELF
 * headers rather than from the machine this process runs on. It reads files and nothing else; what
 * is ordered from it is autoscale.ts's.
 */
import { existsSync } from 'node:fs'
import { open } from 'node:fs/promises'
import { RELEASE_MACHINES } from './bundle-release.ts'

// The System V ABI's ELF header: four magic bytes, the class, the data encoding (1 is
// little-endian, 2 big-endian), and e_machine as two bytes at offset 18. e_machine's values are
// Linux's include/uapi/linux/elf-em.h: EM_X86_64 62 and EM_AARCH64 183.
const ELF_MAGIC = [0x7f, 0x45, 0x4c, 0x46]
const ELF_DATA = 5
const ELF_LITTLE_ENDIAN = 1
const ELF_BIG_ENDIAN = 2
const ELF_MACHINE = 18
const ELF_MACHINES: Readonly<Record<number, string | undefined>> = {
  62: RELEASE_MACHINES.x64,
  183: RELEASE_MACHINES.arm64,
}

/** The released machine an ELF executable is built for, as `uname -m` names it; null for any other file. */
async function elfMachine(path: string): Promise<string | null> {
  const file = await open(path, 'r').catch(() => null)
  if (file === null) return null
  try {
    const header = Buffer.alloc(ELF_MACHINE + 2)
    const { bytesRead } = await file.read(header, 0, header.length, 0)
    if (bytesRead < header.length || ELF_MAGIC.some((byte, index) => header[index] !== byte)) return null
    const encoding = header[ELF_DATA]
    if (encoding !== ELF_LITTLE_ENDIAN && encoding !== ELF_BIG_ENDIAN) return null
    const machine =
      encoding === ELF_LITTLE_ENDIAN ? header.readUInt16LE(ELF_MACHINE) : header.readUInt16BE(ELF_MACHINE)
    return ELF_MACHINES[machine] ?? null
  } finally {
    await file.close()
  }
}

/**
 * The machine the served pryxd is built for, when the served agent, if there is one, is built for
 * the same; null when there is no pryxd to serve or the two differ, so that no host is ordered
 * that couldn't run what it would be handed.
 */
export async function servedMachine(pryxdBin: string, agentBin?: string): Promise<string | null> {
  const pryxd = await elfMachine(pryxdBin)
  if (pryxd === null) return null
  if (agentBin !== undefined && existsSync(agentBin) && (await elfMachine(agentBin)) !== pryxd) return null
  return pryxd
}
