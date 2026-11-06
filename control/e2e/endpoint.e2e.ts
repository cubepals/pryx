// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The node endpoint in a process of its own, for fleet-runtime.e2e.test.ts. It runs on Node, as
 * production runs it: Bun's TLS server demands a client certificate whenever it asks for one, so a
 * node that has none yet couldn't enroll there.
 *
 *   node endpoint.e2e.ts <ca.pem> <ca.key> <port> <deployment> [<listen host> [<ip,ip…>]]
 *
 * DATABASE_URL in the environment, and PRYXD_STATIC_BIN for the pryxd it hands to joining
 * hosts and offers to older nodes (PRYX_UPGRADES=off offers it to none). It listens on 127.0.0.1 unless told otherwise, and its certificate names 127.0.0.1 and
 * the addresses given.
 *
 * It writes `listening` once its port is bound, so a node may dial it the moment it does. A port
 * it can't bind ends it with the error, before it writes anything.
 */
import { readFileSync } from 'node:fs'
import { FleetCa } from '../src/ca.ts'
import { createDb, createPool } from '../src/database.ts'
import { startNodeEndpoint } from '../src/node-endpoint.ts'

const [certFile = '', keyFile = '', port = '', deployment = '', host = '127.0.0.1', ips = ''] =
  process.argv.slice(2)
const pool = createPool(process.env.DATABASE_URL ?? '')
const ca = await FleetCa.fromPem(readFileSync(certFile, 'utf8'), readFileSync(keyFile, 'utf8'))
const endpoint = await startNodeEndpoint({
  db: createDb(pool),
  ca,
  registry: {
    deployment,
    heartbeatSeconds: 1,
    leaseSeconds: 30,
    certDays: 30,
    renewDays: 10,
  },
  listen: { host, port: Number(port) },
  names: { dns: [], ips: ['127.0.0.1', ...ips.split(',').filter(Boolean)] },
  pryxdBin: process.env.PRYXD_STATIC_BIN ?? '/nonexistent/pryxd',
  bootstrapSignatureFile: process.env.PRYX_BOOTSTRAP_SIGNATURE_FILE,
  upgrades: process.env.PRYX_UPGRADES !== 'off',
})
// The handler goes first: a parent may signal the moment it reads `listening`.
process.on('SIGTERM', () => {
  endpoint
    .stop()
    .then(() => pool.end())
    .finally(() => process.exit(0))
})
process.stdout.write('listening\n')
