// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
/** Node enrollment and child processes for a real public API deployment. */
import { mkdirSync, openSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { FleetCa } from '../src/ca.ts'
import type { loadConfig } from '../src/config.ts'
import type { startControl } from '../src/main.ts'
import { createToken } from '../src/registry.ts'
import { nodeConfigFor } from './runtime-config.ts'
import { freePort } from './waits.ts'

export async function spawnNodes(at: {
  name: string
  dir: string
  bin: string
  options: {
    tier?: import('../src/isolation.ts').Isolation
    nodes?: number
    allowedImages?: string[]
    agent?: boolean
    storeLease?: boolean
  }
  pems: Awaited<ReturnType<typeof FleetCa.generate>>
  control: Awaited<ReturnType<typeof startControl>>
  config: ReturnType<typeof loadConfig>
}) {
  const { name, dir, bin, options, pems, control, config } = at
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
        tier: options.tier,
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
  return children
}
