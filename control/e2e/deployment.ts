// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A real control and local pryxd deployment for public API, CLI and edge CI suites. */
import { spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, openSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FleetCa } from '../src/ca.ts'
import { loadConfig } from '../src/config.ts'
import { SECOND_MS } from '../src/durations.ts'
import { createKey } from '../src/keys.ts'
import { startControl } from '../src/main.ts'
import { createToken, nodeSummaries } from '../src/registry.ts'
import { freshDatabase, hasDatabase } from '../test/nodes.ts'
import { nodeConfigFor } from './runtime-config.ts'
import { createTestBucket } from './test-bucket.ts'
import { freePort, until } from './waits.ts'
export const bin = process.env.PRYXD_BIN ?? new URL('../../target/debug/pryxd', import.meta.url).pathname
export const ready = Boolean(
  existsSync(bin) && hasDatabase && process.env.S3_TEST_ENDPOINT && existsSync('/var/run/docker.sock'),
)
export const LEASE_TEST = { seconds: 10, renewSeconds: 3 }
export async function deployment(
  options: {
    allowedImages?: string[]
    storeLease?: boolean
    nodes?: number
    runtimeEndpoint?: string
    agent?: boolean
  } = {},
) {
  const name = `public${randomBytes(4).toString('hex')}`,
    dir = join(tmpdir(), name)
  mkdirSync(dir, { recursive: true })
  const fresh = await freshDatabase(),
    pems = await FleetCa.generate(name)
  const s3 = {
    endpoint: process.env.S3_TEST_ENDPOINT ?? '',
    bucket: name,
    accessKeyId: process.env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
    secretAccessKey: process.env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
  }
  await createTestBucket(s3.endpoint, s3, s3.bucket)
  const operatorToken = randomBytes(32).toString('hex'),
    endpointPort = await freePort()
  const config = loadConfig({
    DATABASE_URL: fresh.url,
    PRYX_DEPLOYMENT: name,
    PRYX_STORE_LEASE: options.storeLease ? 'on' : 'off',
    PRYX_STORE_LEASE_SECONDS: String(LEASE_TEST.seconds),
    PRYX_STORE_LEASE_RENEW_SECONDS: String(LEASE_TEST.renewSeconds),
    PRYX_CA_CERT: pems.certPem,
    PRYX_CA_KEY: pems.keyPem,
    PRYX_OPERATOR_TOKEN: operatorToken,
    PRYX_EDGE_TOKEN: randomBytes(32).toString('hex'),
    PRYX_NODE_LISTEN: `127.0.0.1:${endpointPort}`,
    S3_ENDPOINT: s3.endpoint,
    S3_RUNTIME_ENDPOINT: options.runtimeEndpoint,
    S3_BUCKET: s3.bucket,
    S3_ACCESS_KEY: s3.accessKeyId,
    S3_SECRET_KEY: s3.secretAccessKey,
    PRYX_PUBLIC_DOMAIN: 'pryx.test',
  })
  const control = await startControl({
    ...config,
    apiListen: '127.0.0.1:0',
    operatorListen: '127.0.0.1:0',
    heartbeatSeconds: 1,
    suspend: true,
  })
  const children: { child: Bun.Subprocess; path: string }[] = []
  const count = options.nodes ?? 1
  for (let index = 0; index < count; index++) {
    const nodeDir = count === 1 ? dir : join(dir, String(index))
    mkdirSync(nodeDir, { recursive: true })
    writeFileSync(join(nodeDir, 'ca.pem'), pems.certPem)
    const token = await createToken(control.db, { regionKey: 'default', ttlSeconds: 600, createdBy: 'e2e' })
    writeFileSync(join(nodeDir, 'token'), token.token, { mode: 0o600 })
    const path = join(nodeDir, 'pryxd.toml')
    writeFileSync(
      path,
      nodeConfigFor(name)({
        allowedImages: options.allowedImages,
        agent: options.agent,
        storeLease: options.storeLease,
        home: nodeDir,
        endpointUrl: config.joinUrl,
        dir: nodeDir,
        apiPort: await freePort(),
        opsPort: await freePort(),
        low: 40000 + (Number.parseInt(name.slice(-3), 16) % 90) * 100 + index * 50,
      }),
    )
    const args = [bin, 'serve', '--config', path, '--log-format', 'pretty'],
      log = openSync(join(nodeDir, 'pryxd.log'), 'a')
    const child = Bun.spawn(process.env.PRYXD_SUDO === '1' ? ['sudo', '-n', ...args] : args, {
      stdout: log,
      stderr: log,
    })
    children.push({ child, path })
  }
  await until('public node healthy', async () => {
    const nodes = await nodeSummaries(control.db, { suspectSeconds: 15, unavailableSeconds: 45 })
    return nodes.length === count && nodes.every((n) => n.derivedHealth === 'healthy') ? nodes : null
  })
  await control.runtime.refreshRegistry()
  const credential = await createKey(control.db, 'default', 'e2e')
  return {
    control,
    dir,
    credential,
    children,
    env: {
      ...process.env,
      PRYX_API_URL: control.apiUrl.replace(/\/$/, ''),
      PRYX_API_KEY: credential.key,
      PRYX_OPERATOR_API: control.operatorUrl.replace(/\/$/, ''),
      PRYX_OPERATOR_TOKEN: operatorToken,
      PRYX_EDGE_TOKEN: control.config.edgeToken,
    },
    close: async () => {
      for (const { child, path } of children) {
        const kill = ['pkill', '-TERM', '-f', '--', `serve --config ${path}`]
        spawnSync(
          process.env.PRYXD_SUDO === '1' ? 'sudo' : (kill[0] ?? 'pkill'),
          process.env.PRYXD_SUDO === '1' ? ['-n', ...kill] : kill.slice(1),
        )
        let timer: ReturnType<typeof setTimeout> | undefined
        const exited = await Promise.race([
          child.exited.then(() => true),
          new Promise<false>((resolve) => {
            timer = setTimeout(() => resolve(false), 10 * SECOND_MS)
          }),
        ])
        clearTimeout(timer)
        if (!exited) {
          // Fault suites deliberately suspend nodes in the middle of runtime requests. Cleanup
          // cannot depend on that damaged node completing another request or a graceful drain.
          const force = ['pkill', '-KILL', '-f', '--', `serve --config ${path}`]
          spawnSync(
            process.env.PRYXD_SUDO === '1' ? 'sudo' : (force[0] ?? 'pkill'),
            process.env.PRYXD_SUDO === '1' ? ['-n', ...force] : force.slice(1),
          )
          await child.exited
        }
      }
      const ids = spawnSync('docker', ['ps', '-aq', '--filter', `label=blocklyd.deployment=${name}`])
        .stdout.toString()
        .split(/\s+/)
        .filter(Boolean)
      if (ids.length) spawnSync('docker', ['rm', '-f', ...ids])
      await control.stop()
      await fresh.drop()
      if (process.env.PRYXD_SUDO === '1') spawnSync('sudo', ['-n', 'rm', '-rf', dir])
      else rmSync(dir, { recursive: true, force: true })
    },
  }
}
