// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Bounded polling and free loopback ports for the real-daemon suites. */

import { createServer } from 'node:net'
import { SECOND_MS } from '../src/durations.ts'

export async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      server.close(() => resolve(typeof address === 'object' && address ? address.port : 0))
    })
  })
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
