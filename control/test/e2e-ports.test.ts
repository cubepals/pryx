// SPDX-License-Identifier: FSL-1.1-ALv2
/** Fixture listeners avoid ephemeral outbound and sandbox port pools across repeated contexts. */
import { expect, test } from 'bun:test'
import { createServer } from 'node:net'
import { nodePortLow } from '../e2e/runtime-config.ts'
import { freePort } from '../e2e/waits.ts'

test('fixture listener probes never reuse a port or overlap workload pools', async () => {
  const ports = await Promise.all(Array.from({ length: 64 }, () => freePort()))
  expect(new Set(ports).size).toBe(ports.length)
  for (const port of ports) {
    expect(port).toBeGreaterThan(0)
    expect(port).toBeLessThan(nodePortLow('000', 0))
  }
  const listeners = ports.map(() => createServer())
  try {
    await Promise.all(
      listeners.map(
        (server, index) =>
          new Promise<void>((resolve, reject) => {
            server.once('error', reject)
            server.listen(ports[index], '127.0.0.1', resolve)
          }),
      ),
    )
    expect(listeners.every((server) => server.listening)).toBe(true)
  } finally {
    await Promise.all(
      listeners.map((server) => new Promise<void>((resolve) => server.close(() => resolve()))),
    )
  }
})
