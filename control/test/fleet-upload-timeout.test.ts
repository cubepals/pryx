// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A lost upload acknowledgement must expire before the fleet fixture's test deadline. Uses
 * real Postgres and mutual TLS; the owned node double stores bytes before withholding its reply.
 * This covers export's upload-state wait, not Docker or a particular external network failure. */
import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { createServer } from 'node:http'
import type { AddressInfo } from 'node:net'
import { createServer as createTlsServer, type TLSSocket } from 'node:tls'
import { sql } from 'drizzle-orm'
import { FleetClient } from '../e2e/fleet-client.ts'
import { FleetStore } from '../src/archive-transport.ts'
import { Archives } from '../src/archives.ts'
import { controlPlaneName } from '../src/ca.ts'
import { SECOND_MS } from '../src/durations.ts'
import { encodeSnapshot } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import { getNode } from '../src/registry.ts'
import { Transfers } from '../src/transfers.ts'
import { WorkloadApi } from '../src/workload-api.ts'
import { enrollNode, freshDatabase, hasDatabase, testCa } from './nodes.ts'
import { FlakyStore, Nodes } from './runtime-nodes.ts'

test.skipIf(!hasDatabase)(
  'a withheld upload acknowledgement releases export before the fleet test deadline',
  async () => {
    const { db, drop } = await freshDatabase()
    const store = new FlakyStore()
    await store.start()
    const fleetStore = new FleetStore(store, 'test')
    const ca = await testCa()
    const node = await enrollNode(db, ca, 'held-upload')
    const identity = await ca.identity(controlPlaneName('test'), 'client', 1)
    const client = new FleetClient({ caPem: ca.pem, ...identity }, 'test', SECOND_MS)
    const nodes = new Nodes(store)
    const sockets = new Set<TLSSocket>()
    let requests = 0
    let received!: () => void
    const applied = new Promise<void>((resolve) => {
      received = resolve
    })
    const destinations: string[] = []
    const http = createServer((request, response) => {
      const chunks: Buffer[] = []
      request.on('data', (chunk: Buffer) => chunks.push(chunk))
      request.on('end', async () => {
        const body = JSON.parse(Buffer.concat(chunks).toString())
        destinations.push(new URL(body.url).pathname)
        const sent = await nodes.call(nodeAddress, 'POST', request.url ?? '', { body })
        if (++requests === 1) {
          // The mutation happened. No acknowledgement arrives, so resending it is unsafe.
          received()
          return
        }
        response.setHeader('content-type', 'application/json')
        response.end(JSON.stringify(sent))
      })
    })
    const tls = createTlsServer(
      {
        key: node.keyPem,
        cert: node.serverCertPem,
        ca: ca.pem,
        requestCert: true,
        rejectUnauthorized: true,
      },
      (socket) => {
        sockets.add(socket)
        socket.once('close', () => sockets.delete(socket))
        http.emit('connection', socket)
      },
    )
    await new Promise<void>((resolve) => tls.listen(0, '127.0.0.1', resolve))
    const nodeAddress = { id: node.id, apiAddress: `127.0.0.1:${(tls.address() as AddressInfo).port}` }
    await db.execute(
      sql`UPDATE fleet_nodes SET api_address = ${nodeAddress.apiAddress} WHERE id = ${node.id}`,
    )
    const archives = new Archives({
      db,
      api: new WorkloadApi(client),
      store: fleetStore,
      suspectSeconds: THRESHOLDS.suspectSeconds,
      log: () => {},
    })
    const transfers = new Transfers({
      db,
      api: new WorkloadApi(client),
      store: fleetStore,
      archives,
      provider: 'test',
      log: () => {},
      ledger: {
        nodeOf: async () => {
          const row = await getNode(db, node.id)
          if (!row) throw new Error('The enrolled test node disappeared')
          return row
        },
      },
    })
    const archive = randomUUID(),
      workload = randomUUID()
    await db.execute(sql`INSERT INTO fleet_archives
      (id, workload, purpose, epoch, node_id, local_id, status, consistency, captured_at)
      VALUES (${archive}, ${workload}, 'snapshot', 1, ${node.id}, ${archive}, 'local', 'stopped', now())`)
    const handle = encodeSnapshot({ deployment: 'test', key: workload, archive, epoch: 1 })
    const upload = transfers.upload(archive)
    let exported: ReturnType<Transfers['exportSnapshot']> | undefined
    let deadline: ReturnType<typeof setTimeout> | undefined
    try {
      await applied
      expect(await archives.archiveOf(archive)).toMatchObject({ status: 'uploading' })
      expect(store.objects.get(fleetStore.objectKey(workload, archive))).toEqual(nodes.archive)
      exported = transfers.exportSnapshot(handle, await store.archiveTarget('foreign/copy'))
      const completed = Promise.all([upload, exported])
      const expired = new Promise<never>((_, reject) => {
        deadline = setTimeout(
          () => reject(new Error('Export is still waiting on the unacknowledged upload')),
          5 * SECOND_MS,
        )
      })
      const [status, result] = await Promise.race([completed, expired])
      expect(status).toBe('failed')
      expect(result.sizeBytes).toBe(nodes.archive.length)
      expect(store.objects.get('foreign/copy')).toEqual(nodes.archive)
      expect(await archives.archiveOf(archive)).toMatchObject({ status: 'local', attempts: 1 })
      expect(requests).toBe(2)
      expect(destinations).toEqual([
        `/${encodeURIComponent(fleetStore.objectKey(workload, archive))}`,
        '/foreign%2Fcopy',
      ])
    } finally {
      clearTimeout(deadline)
      client.close()
      for (const socket of sockets) socket.destroy()
      tls.close()
      http.close()
      await Promise.allSettled([upload, ...(exported ? [exported] : [])])
      store.close()
      await drop()
    }
  },
  15 * SECOND_MS,
)
