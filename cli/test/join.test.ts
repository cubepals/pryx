// SPDX-License-Identifier: FSL-1.1-ALv2
/** A fleet CA authenticates transport, never arbitrary host code. */
import { expect, test } from 'bun:test'
import { generateKeyPairSync, sign } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { FleetCa } from '../../control/src/ca.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import {
  bootstrapHash,
  bootstrapOffer,
  bootstrapStatement,
  joinBootstrapScript,
} from '../../control/src/join-bootstrap.ts'
import { joinCommand, joinToken } from '../../control/src/join-token.ts'
import { joinHost } from '../src/join.ts'
import { joinTarget, verifyBootstrapOffer } from '../src/join-trust.ts'

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`

test('a compromised fleet endpoint cannot execute its mutable bootstrap', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-join-trust-'))
  const pems = await FleetCa.generate('bootstrap')
  const ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
  const identity = await ca.identity('bootstrap', 'server', 1, { ips: ['127.0.0.1'] })
  const marker = join(dir, 'untrusted-executed')
  const script = `#!/bin/sh\nprintf compromised > ${quote(marker)}\n`
  const requests: string[] = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    tls: { cert: identity.certPem, key: identity.keyPem },
    fetch(request) {
      const path = new URL(request.url).pathname
      requests.push(path)
      if (path.endsWith('/ca.pem')) return new Response(ca.pem)
      if (path.endsWith('/bootstrap.json'))
        return Response.json({ version: PRYX_VERSION, sha256: '0'.repeat(64) })
      return new Response(script)
    },
  })
  // The host's own CLI, independent of the fleet endpoint.
  writeFileSync(
    join(dir, 'pryx'),
    `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(new URL('../src/main.ts', import.meta.url).pathname)} "$@"\n`,
    { mode: 0o700 },
  )
  const point = { url: `https://127.0.0.1:${server.port}`, caPem: ca.pem, deployment: 'bootstrap' }
  try {
    const child = Bun.spawn(['sh', '-c', joinCommand(point, joinToken(point, 'fixture-secret'))], {
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}` },
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const [code, stderr] = await Promise.all([child.exited, new Response(child.stderr).text()])
    expect(existsSync(marker)).toBe(false)
    expect(code).not.toBe(0)
    expect(stderr).toContain('signature')
    expect(requests).toContain('/fleet/v1/bootstrap.json')
  } finally {
    server.stop(true)
    rmSync(dir, { recursive: true, force: true })
  }
})

async function servedBootstrap() {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-signed-bootstrap-'))
  const pems = await FleetCa.generate('signed-bootstrap')
  const ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
  const identity = await ca.identity('signed-bootstrap', 'server', 1, { ips: ['127.0.0.1'] })
  const keys = generateKeyPairSync('ed25519')
  const publicKey = keys.publicKey.export({ format: 'pem', type: 'spki' }).toString()
  const marker = join(dir, 'binary-executed')
  const binary = Buffer.from(`#!/bin/sh\nprintf executed > ${quote(marker)}\n`)
  const script = joinBootstrapScript(
    readFileSync(new URL('../../control/src/join.sh', import.meta.url), 'utf8'),
    readFileSync(new URL('../../control/src/daemon.json', import.meta.url), 'utf8'),
    bootstrapHash(binary),
    bootstrapHash(binary),
  )
  const state = { script, offer: bootstrapOffer(script), daemon: binary, agent: binary, redirect: false }
  const resign = () => {
    state.offer.signature = sign(
      null,
      Buffer.from(bootstrapStatement(state.offer.version, state.offer.sha256)),
      keys.privateKey,
    ).toString('hex')
  }
  resign()
  const requested: string[] = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    tls: { cert: identity.certPem, key: identity.keyPem },
    fetch(request) {
      const path = new URL(request.url).pathname
      requested.push(path)
      if (path.endsWith('/ca.pem')) return new Response(ca.pem)
      if (path.endsWith('/bootstrap.json'))
        return state.redirect
          ? new Response(null, {
              status: StatusCodes.MOVED_TEMPORARILY,
              headers: { location: '/redirected' },
            })
          : Response.json(state.offer)
      if (path.endsWith('/bootstrap.sh')) return new Response(state.script)
      if (path.endsWith('/blocklyd')) return new Response(state.daemon)
      if (path.endsWith('/pryx-agent')) return new Response(state.agent)
      // A compromised endpoint can forge matching checksums; the signed bootstrap ignores them.
      return new Response(`${bootstrapHash(state.daemon)}  pryxd\n`)
    },
  })
  const point = { url: `https://127.0.0.1:${server.port}`, caPem: ca.pem, deployment: 'signed-bootstrap' }
  return {
    state,
    publicKey,
    marker,
    requested,
    point,
    resign,
    signMessage: (value: string) => sign(null, Buffer.from(value), keys.privateKey).toString('hex'),
    token: joinToken(point, 'fixture-secret'),
    close: () => {
      server.stop(true)
      rmSync(dir, { recursive: true, force: true })
    },
  }
}

test('a signed bootstrap verifies both pinned executables without installing or executing them', async () => {
  const c = await servedBootstrap()
  try {
    await joinHost(c.token, { trustedKeys: c.publicKey, verifyOnly: true })
    expect(c.requested).toContain('/fleet/v1/blocklyd')
    expect(c.requested).toContain('/fleet/v1/pryx-agent')
    expect(c.requested.some((path) => path.endsWith('.sha256'))).toBe(false)
    expect(existsSync(c.marker)).toBe(false)
    expect(c.state.script.indexOf('release asset verification')).toBeLessThan(
      c.state.script.indexOf('apt-get update'),
    )
  } finally {
    c.close()
  }
})

test('modified signed bootstrap bytes never execute', async () => {
  const c = await servedBootstrap()
  try {
    c.state.script += `\nprintf compromised > ${quote(c.marker)}\n`
    await expect(joinHost(c.token, { trustedKeys: c.publicKey, verifyOnly: true })).rejects.toThrow('digest')
    expect(existsSync(c.marker)).toBe(false)
    expect(c.requested).not.toContain('/fleet/v1/blocklyd')
  } finally {
    c.close()
  }
})

for (const asset of ['daemon', 'agent'] as const)
  test(`substituted ${asset} and endpoint checksums cannot change a signed asset pin`, async () => {
    const c = await servedBootstrap()
    try {
      c.state[asset] = Buffer.from(`#!/bin/sh\nprintf malicious > ${quote(c.marker)}\n`)
      await expect(joinHost(c.token, { trustedKeys: c.publicKey, verifyOnly: true })).rejects.toThrow(
        'bootstrap failed',
      )
      expect(existsSync(c.marker)).toBe(false)
      expect(c.requested).toContain('/fleet/v1/blocklyd')
      expect(c.requested).toContain('/fleet/v1/pryx-agent')
    } finally {
      c.close()
    }
  })

test('CA substitution stops before any authenticated bootstrap request', async () => {
  const c = await servedBootstrap()
  try {
    const token = joinToken({ ...c.point, caPem: 'another CA' }, 'fixture-secret')
    await expect(joinHost(token, { trustedKeys: c.publicKey, verifyOnly: true })).rejects.toThrow(
      'pin verification FAILED',
    )
    expect(c.requested).toEqual(['/fleet/v1/ca.pem'])
  } finally {
    c.close()
  }
})

test('bootstrap redirects are refused', async () => {
  const c = await servedBootstrap()
  try {
    c.state.redirect = true
    await expect(joinHost(c.token, { trustedKeys: c.publicKey, verifyOnly: true })).rejects.toThrow(
      'download refused',
    )
    expect(c.requested).not.toContain('/redirected')
  } finally {
    c.close()
  }
})

test('endpoint keys, release rollback and another signing domain cannot authorize bootstrap', async () => {
  const c = await servedBootstrap()
  try {
    expect(() => verifyBootstrapOffer({ ...c.state.offer, trustedKeys: c.publicKey })).toThrow(
      'trusted release key',
    )
    expect(verifyBootstrapOffer(c.state.offer, c.publicKey).sha256).toBe(c.state.offer.sha256)
    c.state.offer.version = '0.0.0'
    c.resign()
    expect(() => verifyBootstrapOffer(c.state.offer, c.publicKey)).toThrow('older')
    c.state.offer.version = PRYX_VERSION
    c.state.offer.signature = c.signMessage(`pryxd release\n${PRYX_VERSION}\n${c.state.offer.sha256}\n`)
    expect(() => verifyBootstrapOffer(c.state.offer, c.publicKey)).toThrow('trusted release key')
    expect(() => joinTarget(joinToken({ ...c.point, url: 'http://127.0.0.1' }, 'fixture-secret'))).toThrow(
      'HTTPS',
    )
  } finally {
    c.close()
  }
})
