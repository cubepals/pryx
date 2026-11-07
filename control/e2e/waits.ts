// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Bounded polling and free loopback ports for the real-daemon suites. */

import { randomInt } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { SECOND_MS } from '../src/durations.ts'
import { nodePortLow } from './runtime-config.ts'

// Closed ephemeral probes can immediately become an outbound connection's source port.
// Keep fixture listeners below both the kernel ephemeral range and workload port pools.
const listenerLow = Math.max(1, Number(readFileSync('/proc/sys/net/ipv4/ip_unprivileged_port_start', 'utf8')))
const listenerHigh = nodePortLow('000', 0) - 1
const allocated = new Set<number>()
const MAX_PORT_PROBES = 64
export async function freePort(): Promise<number> {
  if (listenerHigh <= listenerLow) throw new Error('No room for fixture listeners below workload ports')
  for (let attempt = 0; attempt < MAX_PORT_PROBES; attempt++) {
    const port = randomInt(listenerLow, listenerHigh + 1)
    if (allocated.has(port)) continue
    allocated.add(port)
    const available = await new Promise<boolean>((resolve, reject) => {
      const server = createServer()
      server.once('error', (error: NodeJS.ErrnoException) => {
        if (error.code === 'EADDRINUSE') resolve(false)
        else reject(error)
      })
      server.listen(port, '127.0.0.1', () => server.close(() => resolve(true)))
    })
    if (available) return port
  }
  throw new Error('No unused fixture listener found within the probe budget')
}
export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
export async function until<T>(
  what: string,
  check: () => Promise<T | null | undefined | false>,
  ms = 30 * SECOND_MS,
): Promise<T> {
  const deadline = Date.now() + ms
  for (;;) {
    const value = await check().catch(() => null)
    if (value) return value
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
    await sleep(0.2 * SECOND_MS)
  }
}
