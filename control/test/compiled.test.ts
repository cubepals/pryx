// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The compiled control consumes its packaged migrations/bootstrap assets and hash-only initial key on actual listeners. */
import { describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { request } from 'node:https'
import { createServer } from 'node:net'
import { FleetCa } from '../src/ca.ts'
import { SECOND_MS } from '../src/durations.ts'
import { freshDatabase, hasDatabase } from './nodes.ts'

const root = new URL('../../', import.meta.url).pathname,
  bin = `${root}dist/pryx-control`
async function freePort() {
  const server = createServer()
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = (server.address() as { port: number }).port
  await new Promise<void>((resolve) => server.close(() => resolve()))
  return port
}
describe.skipIf(!hasDatabase || !existsSync(bin))('compiled control assets', () => {
  test(
    'migrates, authenticates the initial key and serves the bootstrap script',
    async () => {
      const fresh = await freshDatabase(),
        pems = await FleetCa.generate('compiled'),
        api = await freePort(),
        operator = await freePort(),
        node = await freePort(),
        key = 'pryx_initial_compiled_test_key_123456789'
      const child = Bun.spawn([bin], {
        env: {
          ...process.env,
          DATABASE_URL: fresh.url,
          PRYX_DEPLOYMENT: 'compiled',
          PRYX_CA_CERT: pems.certPem,
          PRYX_CA_KEY: pems.keyPem,
          PRYX_OPERATOR_TOKEN: 'operator-compiled-test-1234567890123',
          PRYX_BOOTSTRAP_API_KEY: key,
          PRYX_API_LISTEN: `127.0.0.1:${api}`,
          PRYX_OPERATOR_LISTEN: `127.0.0.1:${operator}`,
          PRYX_NODE_LISTEN: `127.0.0.1:${node}`,
          PRYX_ASSET_DIR: `${root}target/release-assets/control`,
          S3_ENDPOINT: 'http://127.0.0.1:9000',
          S3_BUCKET: 'test',
          S3_ACCESS_KEY: 'test',
          S3_SECRET_KEY: 'test',
        },
        stdout: 'pipe',
        stderr: 'pipe',
      })
      try {
        const deadline = performance.now() + 10 * SECOND_MS
        let response: Response | undefined
        while (performance.now() < deadline) {
          try {
            response = await fetch(`http://127.0.0.1:${api}/v1/sandboxes`, {
              headers: { authorization: `Bearer ${key}` },
            })
            if (response.ok) break
          } catch {}
          await Bun.sleep(SECOND_MS / 10)
        }
        expect(response?.ok).toBe(true)
        expect(await response?.json()).toEqual({ sandboxes: [] })
        const script = await new Promise<string>((resolve, reject) => {
          const req = request(
            { host: '127.0.0.1', port: node, path: '/fleet/v1/join.sh', ca: pems.certPem },
            (r) => {
              let text = ''
              r.on('data', (d) => {
                text += d
              })
              r.on('end', () => resolve(text))
            },
          )
          req.on('error', reject)
          req.end()
        })
        expect(script).toContain('pryxd join')
        expect(script).not.toContain('@DAEMON_JSON@')
      } finally {
        child.kill('SIGTERM')
        await child.exited
        await fresh.drop()
      }
    },
    15 * SECOND_MS,
  )
})
