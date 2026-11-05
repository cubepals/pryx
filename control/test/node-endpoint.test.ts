// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The node endpoint is bound once it says it listens: `startNodeEndpoint` resolves only then, and
 * endpoint.e2e.ts writes `listening` only then, so a node dialling it at once, as `pryxd join`
 * does right after a restart, finds it there. A port it can't bind fails the start.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { connect, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { FleetCa, nodeName } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { type NodeEndpointOptions, startNodeEndpoint } from '../src/node-endpoint.ts'
import { freshDatabase, hasDatabase } from './nodes.ts'

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      server.close(() => resolve(typeof address === 'object' && address ? address.port : 0))
    })
  })
}

/** Whether something accepts a connection on `port` right now. */
const accepts = (port: number) =>
  new Promise<boolean>((resolve) => {
    const socket = connect(port, '127.0.0.1')
    socket.once('connect', () => {
      socket.destroy()
      resolve(true)
    })
    socket.once('error', () => resolve(false))
  })

describe.skipIf(!hasDatabase)('the node endpoint listens once it is bound', () => {
  const dir = mkdtempSync(join(tmpdir(), 'blockly-endpoint-'))
  let db: Db
  let database: string
  let drop: () => Promise<void>
  let ca: FleetCa

  beforeAll(async () => {
    const fresh = await freshDatabase()
    ;({ db, drop } = fresh)
    database = fresh.url
    const pems = await FleetCa.generate('endpoint')
    ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
    writeFileSync(join(dir, 'ca.pem'), pems.certPem)
    writeFileSync(join(dir, 'ca.key'), pems.keyPem, { mode: 0o600 })
  })

  afterAll(async () => {
    await drop?.()
  })

  const options = (port: number): NodeEndpointOptions => ({
    db,
    ca,
    registry: {
      deployment: 'endpoint',
      heartbeatSeconds: 1,
      leaseSeconds: 30,
      certDays: 30,
      renewDays: 10,
    },
    listen: { host: '127.0.0.1', port },
    names: { dns: [], ips: ['127.0.0.1'] },
    pryxdBin: '/nonexistent/pryxd',
    upgrades: false,
  })

  test('Bun TLS requests verify the CA, client usage and node identity before registry access', async () => {
    const port = await freePort()
    const endpoint = await startNodeEndpoint(options(port))
    const id = crypto.randomUUID()
    const client = await ca.identity(nodeName(id, 'endpoint'), 'client', 1)
    const wrongName = await ca.identity(nodeName(crypto.randomUUID(), 'endpoint'), 'client', 1)
    const wrongUsage = await ca.identity(nodeName(id, 'endpoint'), 'server', 1)
    const otherPems = await FleetCa.generate('other')
    const otherCa = await FleetCa.fromPem(otherPems.certPem, otherPems.keyPem)
    const untrusted = await otherCa.identity(nodeName(id, 'endpoint'), 'client', 1)
    const request = (identity?: { certPem: string; keyPem: string }) =>
      fetch(`https://127.0.0.1:${port}/fleet/v1/nodes/${id}/heartbeat`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', connection: 'close' },
        body: JSON.stringify({ nodeId: id, workloads: [] }),
        tls: { ca: ca.pem, ...(identity ? { cert: identity.certPem, key: identity.keyPem } : {}) },
      })
    try {
      expect((await request()).status).toBe(StatusCodes.UNAUTHORIZED)
      expect((await request(untrusted)).status).toBe(StatusCodes.UNAUTHORIZED)
      expect((await request(wrongName)).status).toBe(StatusCodes.FORBIDDEN)
      expect((await request(wrongUsage)).status).toBe(StatusCodes.UNAUTHORIZED)
      const valid = await request(client)
      expect(valid.status).toBe(StatusCodes.NOT_FOUND)
      expect((await valid.json()).error.code).toBe('unknown_node')
    } finally {
      await endpoint.stop()
    }
  })

  /** endpoint.e2e.ts on `port`, and what it writes to stdout until it says it listens or exits. */
  async function spawnEndpoint(port: number) {
    const args = [join(dir, 'ca.pem'), join(dir, 'ca.key'), `${port}`, 'endpoint']
    const child = Bun.spawn(['node', join(import.meta.dir, '../e2e/endpoint.e2e.ts'), ...args], {
      env: { ...process.env, DATABASE_URL: database, PRYX_UPGRADES: 'off' },
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const reader = (child.stdout as ReadableStream<Uint8Array>).getReader()
    let said = ''
    while (!said.includes('listening')) {
      const { value, done } = await reader.read()
      if (done) break
      said += new TextDecoder().decode(value)
    }
    reader.releaseLock()
    return { child, said }
  }

  test('restarted on a port the endpoint before it still holds, the start fails with why', async () => {
    const port = await freePort()
    const first = await startNodeEndpoint(options(port))
    expect(await accepts(port)).toBe(true)
    await expect(startNodeEndpoint(options(port))).rejects.toMatchObject({
      code: 'EADDRINUSE',
    })
    await first.stop()
    const again = await startNodeEndpoint(options(port))
    expect(await accepts(port)).toBe(true)
    await again.stop()
  })

  test(
    'restarted in a process of its own, it is there to dial the moment it says it listens',
    async () => {
      const port = await freePort()
      for (let round = 0; round < 5; round++) {
        const { child, said } = await spawnEndpoint(port)
        expect(said).toContain('listening')
        expect(await accepts(port)).toBe(true)
        child.kill('SIGTERM')
        expect(await child.exited).toBe(0)
      }
    },
    MINUTE_MS,
  )

  test(
    'in a process of its own, a port another holds ends it before it says it listens',
    async () => {
      const port = await freePort()
      const first = await spawnEndpoint(port)
      expect(first.said).toContain('listening')
      const second = await spawnEndpoint(port)
      expect(second.said).not.toContain('listening')
      expect(await second.child.exited).not.toBe(0)
      expect(await new Response(second.child.stderr as ReadableStream).text()).toContain('EADDRINUSE')
      first.child.kill('SIGTERM')
      await first.child.exited
    },
    MINUTE_MS,
  )
})
