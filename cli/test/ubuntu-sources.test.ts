// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise the Ubuntu fixture's source rewrite and package preparation with disposable APT sources.
 * The fake package manager records mirror selection without downloading packages. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

test('Ubuntu fixture preserves signed legacy and Deb822 archive sources', () => {
  const script = readFileSync(new URL('../../deploy/container-acceptance.sh', import.meta.url), 'utf8')
  const rewrite = script.match(/^rewrite_ubuntu_sources\(\) \{\n[\s\S]*?^\}/m)?.[0]
  if (!rewrite) throw new Error('The Ubuntu source preparation function is missing')
  const root = mkdtempSync(join(tmpdir(), 'pryx-ubuntu-sources-'))
  const sources = join(root, 'etc/apt/sources.list.d')
  mkdirSync(sources, { recursive: true })
  const legacy = join(root, 'etc/apt/sources.list')
  const modern = join(sources, 'ubuntu.sources')
  const mirror = 'https://mirrors.edge.kernel.org/ubuntu'
  const beforeLegacy =
    '# retain comment\ndeb [arch=amd64 signed-by=/key.gpg] http://archive.ubuntu.com/ubuntu noble main universe\ndeb-src http://security.ubuntu.com/ubuntu noble-security main\n'
  const beforeModern =
    'Types: deb\nURIs: http://archive.ubuntu.com/ubuntu\nSuites: noble noble-updates\nComponents: main universe\nSigned-By: /key.gpg\n'
  const run = () =>
    Bun.spawnSync([
      '/bin/sh',
      '-eu',
      '-c',
      `${rewrite}\nrewrite_ubuntu_sources "$@"`,
      'sources',
      root,
      mirror,
    ])
  try {
    writeFileSync(legacy, beforeLegacy)
    expect(run().exitCode).toBe(0)
    expect(readFileSync(legacy, 'utf8')).toBe(beforeLegacy.replaceAll(/http:\/\/[^\s]+/g, mirror))
    writeFileSync(modern, beforeModern)
    expect(run().exitCode).toBe(0)
    expect(readFileSync(modern, 'utf8')).toBe(
      beforeModern.replace('http://archive.ubuntu.com/ubuntu', mirror),
    )
    rmSync(legacy)
    rmSync(modern)
    const refused = run()
    expect(refused.exitCode).not.toBe(0)
    expect(refused.stderr.toString()).toContain('no recognized signed archive sources')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

const sessionPackages = ['dbus-user-session', 'libpam-systemd', 'polkitd']
const basePackages = ['curl', 'ca-certificates', 'python3', 'passwd', 'tar']
const mirrors = ['https://first.invalid/ubuntu', 'https://second.invalid/ubuntu'] as const

function prepareUbuntu(booted: boolean, sessionMirror: string) {
  const script = readFileSync(new URL('../../deploy/container-acceptance.sh', import.meta.url), 'utf8')
  const setupStart = script.indexOf('\nif command -v apt-get >/dev/null; then')
  if (setupStart < 0) throw new Error('Cannot isolate fixture functions from host setup')
  const functions = script.slice(0, setupStart)
  const root = mkdtempSync(join(tmpdir(), 'pryx-ubuntu-install-'))
  try {
    mkdirSync(join(root, 'etc/apt'), { recursive: true })
    const sources = join(root, 'etc/apt/sources.list')
    writeFileSync(sources, 'deb http://archive.ubuntu.com/ubuntu noble main\n')
    const requests = join(root, 'requests')
    writeFileSync(requests, '')
    writeFileSync(
      join(root, 'apt-get'),
      `#!/bin/sh
set -eu
mirror=$(awk '$1 == "deb" { print $2 }' "$SOURCES")
echo "$mirror $*" >> "$REQUESTS"
for arg do
  if [ "$arg" = dbus-user-session ] && [ "$mirror" != "$SESSION_MIRROR" ]; then
    exit 100
  fi
done
`,
      { mode: 0o755 },
    )
    const run = Bun.spawnSync(
      ['/bin/sh', '-eu', '-c', `${functions}\ninstall_ubuntu_prerequisites "$1"`, 'apt', root],
      {
        env: {
          ...process.env,
          PATH: `${root}:${process.env.PATH}`,
          PRYX_PREPARE_ONLY: booted ? '1' : '0',
          PRYX_TEST_UBUNTU_MIRRORS: mirrors.join(' '),
          SOURCES: sources,
          REQUESTS: requests,
          SESSION_MIRROR: sessionMirror,
        },
      },
    )
    const calls = readFileSync(requests, 'utf8')
      .trim()
      .split('\n')
      .map((line) => line.split(' '))
    const packagesAt = (mirror: string) =>
      calls.filter((call) => call[0] === mirror && call.includes('install')).flat()
    return { exitCode: run.exitCode, packagesAt }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

test('booted Ubuntu tries the next mirror when a session package download fails', () => {
  const { exitCode, packagesAt } = prepareUbuntu(true, mirrors[1])
  expect(exitCode).toBe(0)
  for (const mirror of mirrors) {
    expect(packagesAt(mirror)).toEqual(expect.arrayContaining([...basePackages, ...sessionPackages]))
  }
})

test('booted Ubuntu fails when every mirror lacks a session package', () => {
  const { exitCode, packagesAt } = prepareUbuntu(true, '')
  expect(exitCode).not.toBe(0)
  for (const mirror of mirrors) expect(packagesAt(mirror)).toContain('dbus-user-session')
})

test('ordinary Ubuntu containers require only their base packages', () => {
  const { exitCode, packagesAt } = prepareUbuntu(false, '')
  expect(exitCode).toBe(0)
  const packages = packagesAt(mirrors[0])
  expect(packages).toEqual(expect.arrayContaining(basePackages))
  for (const pkg of sessionPackages) expect(packages).not.toContain(pkg)
  expect(packagesAt(mirrors[1])).toEqual([])
})
