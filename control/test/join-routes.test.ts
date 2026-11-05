// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What a joining host fetches from the node endpoint (join-routes.ts), as join.sh and `pryxd
 * join` read it, and the release the upgrade rollout reads from the same binary.
 */

import { describe, expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { joinRoutes, pryxdRelease } from '../src/join-routes.ts'
import { caSha256 } from '../src/join-token.ts'
import type { UpgradeOffer } from '../src/wire.ts'

describe('the join routes', () => {
  const dir = mkdtempSync(join(tmpdir(), 'blockly-join-'))
  const caPem = '-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n'
  const get = (pryxdBin: string, path: string, release: UpgradeOffer | null = null) =>
    joinRoutes({ caPem, pryxdBin, release }).request(`https://endpoint/fleet/v1/${path}`)

  test('the CA is served byte for byte, so the token’s hash checks it', async () => {
    const served = await (await get('', 'ca.pem')).text()
    expect(caSha256(served)).toBe(caSha256(caPem))
  })

  test('join.sh carries daemon.json and is a script sh reads', async () => {
    const script = await (await get('', 'join.sh')).text()
    const daemonJson = readFileSync(new URL('../src/daemon.json', import.meta.url), 'utf8')
    expect(script).toContain(daemonJson.trimEnd())
    expect(script).not.toContain('@DAEMON_JSON@')
    expect(script).not.toContain('@AGENT_INSTALL@')
    writeFileSync(join(dir, 'join.sh'), script)
    expect(spawnSync('sh', ['-n', join(dir, 'join.sh')]).status).toBe(0)
  })

  test('pryxd and its sha256, in the form sha256sum -c reads', async () => {
    const bin = join(dir, 'pryxd')
    writeFileSync(bin, 'a static pryxd')
    const binary = Buffer.from(await (await get(bin, 'blocklyd')).arrayBuffer())
    expect(binary.toString()).toBe('a static pryxd')
    const sum = await (await get(bin, 'blocklyd.sha256')).text()
    expect(sum).toBe(`${createHash('sha256').update(binary).digest('hex')}  pryxd\n`)
  })

  test('joining hosts receive the agent and its verified checksum before enrollment', async () => {
    const agentBin = join(dir, 'pryx-agent')
    writeFileSync(agentBin, 'static agent fixture')
    const routes = joinRoutes({ caPem, pryxdBin: '', agentBin, release: null })
    const binary = Buffer.from(
      await (await routes.request('https://endpoint/fleet/v1/pryx-agent')).arrayBuffer(),
    )
    expect(binary.toString()).toBe('static agent fixture')
    expect(await (await routes.request('https://endpoint/fleet/v1/pryx-agent.sha256')).text()).toBe(
      `${createHash('sha256').update(binary).digest('hex')}  pryx-agent\n`,
    )
    const script = await (await routes.request('https://endpoint/fleet/v1/join.sh')).text()
    expect(script.indexOf('pryx-agent.sha256')).toBeLessThan(script.indexOf('/usr/local/bin/pryxd join'))
    expect(script).toContain('sha256sum -c --quiet pryx-agent.sha256')
    writeFileSync(join(dir, 'agent-join.sh'), script)
    expect(spawnSync('sh', ['-n', join(dir, 'agent-join.sh')]).status).toBe(0)
  })

  test('without a pryxd, each answers 404 and says why', async () => {
    for (const path of ['blocklyd', 'blocklyd.sha256', 'blocklyd.json']) {
      const answer = await get(join(dir, 'missing'), path)
      expect(answer.status).toBe(StatusCodes.NOT_FOUND)
      expect(await answer.text()).toContain('PRYX_PRYXD_BIN')
    }
  })

  test('the release the rollout offers is the binary’s own version and sha256, or none', async () => {
    const bin = join(dir, 'versioned')
    writeFileSync(bin, '#!/bin/sh\necho "pryxd 0.3.0"\n')
    chmodSync(bin, 0o755)
    expect(await pryxdRelease(bin)).toEqual({
      version: '0.3.0',
      sha256: createHash('sha256').update(readFileSync(bin)).digest('hex'),
    })
    expect(await pryxdRelease(join(dir, 'missing'))).toBeNull()
    const broken = join(dir, 'broken')
    writeFileSync(broken, '#!/bin/sh\nexit 1\n')
    chmodSync(broken, 0o755)
    expect(await pryxdRelease(broken)).toBeNull()
  })

  test('the release’s signature beside the binary is passed on, in hex, and nothing else is', async () => {
    const bin = join(dir, 'signed')
    writeFileSync(bin, '#!/bin/sh\necho "pryxd 0.3.0"\n')
    chmodSync(bin, 0o755)
    const signature = Buffer.alloc(64, 7)
    writeFileSync(`${bin}.sig`, signature)
    const release = await pryxdRelease(bin)
    expect(release).toEqual({
      version: '0.3.0',
      sha256: createHash('sha256').update(readFileSync(bin)).digest('hex'),
      signature: signature.toString('hex'),
    })
    // What `pryxd upgrade` asks for.
    expect(await (await get(bin, 'blocklyd.json', release)).json()).toEqual(release)
    writeFileSync(`${bin}.sig`, 'not a signature')
    expect(await pryxdRelease(bin)).not.toHaveProperty('signature')
  })
})
