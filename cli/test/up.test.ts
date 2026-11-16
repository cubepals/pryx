// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Installer checks are read-only; file and credential operations are idempotent and checksum failures stop installation. */
import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import vmImages from '../../templates/vm-tools/image-inputs.json'
import { runInstallCommand, up } from '../src/up.ts'
import {
  FLEET_PRYXD_BIN,
  installBinary,
  OPTIONAL_RELEASE_FILES,
  RELEASE_FILES,
  releaseFiles,
} from '../src/up-assets.ts'
import { configurations } from '../src/up-config.ts'
import { readEnvironment, writeChanged, writeEnvironment } from '../src/up-files.ts'
import { gvisorRuntimes, hostInstallProblem, installGvisor } from '../src/up-host.ts'
import { defaultAllowedImages, refreshDefaultImages, replaceDefaultImages } from '../src/up-images.ts'
import { container, service } from '../src/up-units.ts'

test('unsupported distributions name missing packages before installation', () => {
  expect(hostInstallProblem(false, false, 'fedora')).toContain('Missing Docker and nftables')
  expect(hostInstallProblem(true, false, 'fedora')).toContain('Missing nftables')
  expect(hostInstallProblem(true, true, 'fedora')).toBeNull()
  expect(hostInstallProblem(false, false, 'debian')).toBeNull()
})

test('the server preset registers runsc beside existing runtimes and refuses a changed release', async () => {
  expect(gvisorRuntimes({ runtimes: { crun: { path: '/usr/bin/crun' } } }, '/opt/pryx/gvisor')).toEqual({
    crun: { path: '/usr/bin/crun' },
    runsc: { path: '/opt/pryx/gvisor/runsc' },
  })
  const dir = mkdtempSync(join(tmpdir(), 'pryx-gvisor-'))
  const server = Bun.serve({ port: 0, fetch: () => new Response('not the pinned release') })
  const ran: string[][] = []
  try {
    const install = installGvisor(async (args) => void ran.push(args), join(dir, 'gvisor'), `${server.url}x`)
    await expect(install).rejects.toThrow('does not match its pinned SHA-512')
    expect(ran).toEqual([])
    expect(() => statSync(join(dir, 'gvisor'))).toThrow()
  } finally {
    server.stop(true)
    rmSync(dir, { recursive: true, force: true })
  }
})

test('check mode reports prerequisites without generating credentials', async () => {
  const result = await up(['--check', '--domain', 'pryx.test'])
  expect(JSON.stringify(result)).not.toContain('PRYX_API_KEY')
  expect(result).toHaveProperty('domain', 'pryx.test')
})
test('root-only environment files and generated secrets are stable on a second run', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-up-'))
  try {
    const first = await configurations('pryx.test', dir),
      second = await configurations('pryx.test', dir)
    expect(first.fresh).toBe(true)
    expect(second.fresh).toBe(false)
    expect(second.changed).toBe(false)
    expect(second.edgeChanged).toBe(false)
    expect(second.control.PRYX_CA_KEY).toBe(first.control.PRYX_CA_KEY)
    for (const name of ['control', 'edge', 'store', 'postgres'])
      expect(statSync(join(dir, `${name}.env`)).mode & 0o777).toBe(0o600)
    writeEnvironment(join(dir, 'escaped.env'), { VALUE: 'quotes" backslash\\ newline\n dollar$ backtick`' })
    expect(readEnvironment(join(dir, 'escaped.env')).VALUE).toBe(
      'quotes" backslash\\ newline\n dollar$ backtick`',
    )
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
test('fleet artifacts migrate the managed path and preserve custom paths and credentials', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-up-migration-'))
  try {
    const first = await configurations('pryx.test', dir)
    expect(first.control.PRYX_PRYXD_BIN).toBe(FLEET_PRYXD_BIN)
    const file = join(dir, 'control.env')
    for (const old of [undefined, '/usr/local/bin/pryxd']) {
      const previous = { ...first.control }
      if (old) previous.PRYX_PRYXD_BIN = old
      else delete previous.PRYX_PRYXD_BIN
      writeEnvironment(file, previous)
      const migrated = await configurations('pryx.test', dir)
      expect(migrated.control.PRYX_PRYXD_BIN).toBe(FLEET_PRYXD_BIN)
      expect(migrated.control.PRYX_CA_KEY).toBe(first.control.PRYX_CA_KEY)
      expect(migrated.control.PRYX_OPERATOR_TOKEN).toBe(first.control.PRYX_OPERATOR_TOKEN)
    }
    writeEnvironment(file, { ...first.control, PRYX_PRYXD_BIN: '/srv/releases/pryxd' })
    expect((await configurations('pryx.test', dir)).control.PRYX_PRYXD_BIN).toBe('/srv/releases/pryxd')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('release checks preserve signature bytes and refuse a modified executable', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-up-assets-'))
  try {
    for (const name of RELEASE_FILES)
      writeFileSync(
        join(dir, name),
        name === 'pryxd.sig' ? Buffer.from(Array.from({ length: 64 }, (_, i) => i + 128)) : Buffer.from(name),
      )
    const lines = RELEASE_FILES.map(
      (n) =>
        `${createHash('sha256')
          .update(readFileSync(join(dir, n)))
          .digest('hex')}  ${n}`,
    )
    writeFileSync(join(dir, 'SHA256SUMS'), lines.join('\n') + '\n')
    expect(await releaseFiles(dir)).toBe(dir)
    const destination = join(dir, 'installed')
    expect(installBinary(join(dir, 'pryxd'), destination)).toBe(true)
    const inode = statSync(destination).ino
    expect(installBinary(join(dir, 'pryxd'), destination)).toBe(false)
    expect(statSync(destination).ino).toBe(inode)
    const signature = readFileSync(join(dir, 'pryxd.sig'))
    writeChanged(join(dir, 'copied.sig'), signature, 0o644)
    expect(readFileSync(join(dir, 'copied.sig'))).toEqual(signature)
    writeFileSync(join(dir, 'pryxd'), 'modified')
    await expect(releaseFiles(dir)).rejects.toThrow('SHA256')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
test('a listed VM descriptor is verified, and a release without one still installs', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-up-optional-'))
  try {
    for (const name of RELEASE_FILES) writeFileSync(join(dir, name), Buffer.from(name))
    const line = (name: string) =>
      `${createHash('sha256')
        .update(readFileSync(join(dir, name)))
        .digest('hex')}  ${name}`
    writeFileSync(join(dir, 'SHA256SUMS'), `${RELEASE_FILES.map(line).join('\n')}\n`)
    expect(await releaseFiles(dir)).toBe(dir)
    for (const name of OPTIONAL_RELEASE_FILES) writeFileSync(join(dir, name), '{"descriptor":true}')
    writeFileSync(
      join(dir, 'SHA256SUMS'),
      `${[...RELEASE_FILES, ...OPTIONAL_RELEASE_FILES].map(line).join('\n')}\n`,
    )
    expect(await releaseFiles(dir)).toBe(dir)
    writeFileSync(join(dir, 'vm-bundle.json'), '{"descriptor":"changed"}')
    await expect(releaseFiles(dir)).rejects.toThrow('vm-bundle.json failed SHA256')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
test('a server install admits the VM bundle images by digest only with its descriptor', () => {
  expect(defaultAllowedImages(false)).toEqual(['alpine:', 'node:'])
  const pinned = defaultAllowedImages(true).slice(2)
  expect(pinned).toEqual(vmImages.map((image) => image.reference))
  for (const reference of pinned) expect(reference).toMatch(/^[a-z0-9./-]+@sha256:[0-9a-f]{64}$/)
})

test('a later server run moves only an unchanged default allowlist to the new pins', () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-images-'))
  try {
    const previous = ['alpine:', 'node:', `docker.io/library/alpine@sha256:${'1'.repeat(64)}`]
    const next = defaultAllowedImages(true)
    const node = (images: string[]) =>
      `deployment_id = "d"\nruntime = "auto"\n\n[workloads]\nallowed_images = ${JSON.stringify(images)}\n\n[fleet]\nurl = "u"\n`
    const config = join(dir, 'pryxd.toml')
    const record = join(dir, 'node-images.json')
    writeFileSync(record, JSON.stringify({ defaults: previous }))
    writeFileSync(config, node(previous))
    expect(refreshDefaultImages(next, config, record)).toBe(true)
    expect(readFileSync(config, 'utf8')).toBe(node(next))
    expect(JSON.parse(readFileSync(record, 'utf8'))).toEqual({ defaults: next })
    expect(refreshDefaultImages(next, config, record)).toBe(false)
    // Removed, narrowed, replaced or reordered by the operator: the list is theirs.
    for (const own of [
      ['alpine:', 'node:'],
      ['alpine:'],
      ['alpine:', 'node:', `docker.io/library/alpine@sha256:${'2'.repeat(64)}`],
      [previous[2] as string, 'alpine:', 'node:'],
      [],
    ]) {
      expect(replaceDefaultImages(node(own), previous, next)).toBeUndefined()
      writeFileSync(record, JSON.stringify({ defaults: previous }))
      writeFileSync(config, node(own))
      expect(refreshDefaultImages(next, config, record)).toBe(false)
      expect(readFileSync(config, 'utf8')).toBe(node(own))
    }
    // No record: an install that named --allow-image, or one older than the record.
    rmSync(record)
    writeFileSync(config, node(previous))
    expect(refreshDefaultImages(next, config, record)).toBe(false)
    expect(readFileSync(config, 'utf8')).toBe(node(previous))
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
test('units read secrets from protected files and expose storage only on loopback', () => {
  expect(service('control')).toContain('EnvironmentFile=/etc/pryx/control.env')
  expect(service('edge')).toContain('CAP_NET_BIND_SERVICE')
  expect(container('postgres', '/usr/bin/docker')).toContain('127.0.0.1:5432:5432')
  expect(container('store', '/usr/bin/docker')).toContain('127.0.0.1:9000:9000')
  expect(container('postgres', '/usr/bin/docker')).not.toContain('POSTGRES_PASSWORD=')
})

test('failed install commands retain both output streams and redact enrollment tokens', async () => {
  const token = 'synthetic-enrollment-secret'
  const failure = await runInstallCommand(
    [
      process.execPath,
      '-e',
      'console.log("doctor stdout " + process.argv[1]); console.error("join stderr " + process.argv[1]); process.exit(1)',
      token,
    ],
    true,
    [token],
  ).then(
    () => '',
    (error) => String(error),
  )
  expect(failure).toContain('doctor stdout [redacted]')
  expect(failure).toContain('join stderr [redacted]')
  expect(failure).not.toContain(token)
})
