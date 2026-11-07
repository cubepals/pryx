// SPDX-License-Identifier: FSL-1.1-ALv2
/** Fixture listeners avoid ephemeral outbound and sandbox port pools across repeated contexts. */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { nodeConfigFor, nodePortLow, TEST_NODE_PORTS } from '../e2e/runtime-config.ts'
import { freePort } from '../e2e/waits.ts'

test('daemon fixtures derive workload pools below the kernel ephemeral range', () => {
  const ephemeralStart = Number(
    readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim().split(/\s+/)[0],
  )
  const deployment = 'e2efe1292'
  for (const index of [0, 1]) {
    const config = nodeConfigFor(deployment)({
      home: '/tmp/fixture',
      dir: '/tmp/fixture',
      endpointUrl: 'https://node.test',
      apiPort: 1001,
      opsPort: 1002,
      // A JavaScript caller's obsolete range must not overlap outbound connections.
      ...{ low: ephemeralStart },
      index,
    })
    const parsed = Bun.TOML.parse(config) as { network: { port_range: [number, number] } }
    const [low, high] = parsed.network.port_range
    expect(low).toBe(nodePortLow(deployment, index))
    expect(high - low + 1).toBe(TEST_NODE_PORTS)
    expect(high).toBeLessThan(ephemeralStart)
  }
})

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
