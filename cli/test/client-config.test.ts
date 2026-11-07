// SPDX-License-Identifier: FSL-1.1-ALv2
/** Private CLI credential loading, atomic replacement and checksum rejection before installer execution. */
import { expect, test } from 'bun:test'
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { readClientConfig, writeClientConfig } from '../src/client-config.ts'

test('client credentials stay private and refuse symlinks or public permissions', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-client-'))
  const path = join(directory, 'config/client.json')
  try {
    expect(readClientConfig(path)).toBeUndefined()
    writeClientConfig({ url: 'http://127.0.0.1:8082', key: 'first' }, path)
    expect(statSync(path).mode & 0o777).toBe(0o600)
    expect(readClientConfig(path)?.key).toBe('first')
    writeClientConfig({ url: 'http://127.0.0.1:8082', key: 'second' }, path)
    expect(readClientConfig(path)?.key).toBe('second')
    chmodSync(path, 0o644)
    expect(() => readClientConfig(path)).toThrow('owner-only')
    chmodSync(path, 0o600)
    const link = join(directory, 'link.json')
    symlinkSync(path, link)
    expect(() => readClientConfig(link)).toThrow()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
test('installer rejects a wrong checksum before invoking the downloaded CLI', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-install-'))
  const sentinel = join(directory, 'executed')
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request) {
      return new Response(
        new URL(request.url).pathname.endsWith('/SHA256SUMS')
          ? `${'0'.repeat(64)}  pryx\n`
          : `#!/bin/sh\ntouch '${sentinel}'\n`,
      )
    },
  })
  try {
    const script = readFileSync('deploy/install.sh', 'utf8')
    const child = Bun.spawn(['sh', '-s'], {
      env: { ...process.env, PRYX_RELEASE_BASE_URL: server.url.toString().replace(/\/$/, '') },
      stdin: new TextEncoder().encode(script),
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const [stdout, stderr, exit] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    expect(exit).not.toBe(0)
    expect(`${stdout}${stderr}`).toContain('checksum')
    expect(existsSync(sentinel)).toBe(false)
  } finally {
    server.stop(true)
    rmSync(directory, { recursive: true, force: true })
  }
})
test('installer rejects URL userinfo disguised as a loopback port', async () => {
  const child = Bun.spawn(['sh', 'deploy/install.sh'], {
    env: { ...process.env, PRYX_RELEASE_BASE_URL: 'http://127.0.0.1:9876@external.invalid/release' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  expect(code).not.toBe(0)
  expect(`${stdout}${stderr}`).toContain('Loopback HTTP release URL needs a numeric port')
})
