// SPDX-License-Identifier: FSL-1.1-ALv2
/** New image listeners stay below the kernel's outgoing ephemeral range without sysctl access.
 * Stored pools remain stable across restarts, including the old implicit daemon default. */
import { closeSync, constants, fstatSync, openSync, readFileSync } from 'node:fs'
import nodeSchema from '../openapi/node-reads.openapi.json'

export function imagePortRange(path: string): [number, number] {
  const previous = nodeSchema['x-pryx-network'].defaultPortRange as [number, number]
  let fd: number | undefined
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
  }
  if (fd !== undefined) {
    try {
      const info = fstatSync(fd)
      if (!info.isFile() || info.uid !== process.getuid?.() || info.mode & 0o077)
        throw new Error('Unsafe image node configuration')
      const config = Bun.TOML.parse(readFileSync(fd, 'utf8')) as {
        network?: { port_range?: [number, number] }
      }
      return config.network?.port_range ?? [...previous]
    } finally {
      closeSync(fd)
    }
  }
  const ephemeral = Number(
    readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim().split(/\s+/)[0],
  )
  const floor = Number(readFileSync('/proc/sys/net/ipv4/ip_unprivileged_port_start', 'utf8'))
  const width = previous[1] - previous[0] + 1
  const low = ephemeral - width
  if (!Number.isInteger(ephemeral) || !Number.isInteger(floor) || low < floor)
    throw new Error('Kernel port policy has no room for the image listener pool')
  return [low, ephemeral - 1]
}
