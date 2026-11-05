// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Installer checks are read-only; file and credential operations are idempotent and checksum failures stop installation. */
import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { runInstallCommand, up } from '../src/up.ts'
import { installBinary, RELEASE_FILES, releaseFiles } from '../src/up-assets.ts'
import { configurations } from '../src/up-config.ts'
import { readEnvironment, writeChanged, writeEnvironment } from '../src/up-files.ts'
import { container, service } from '../src/up-units.ts'

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
