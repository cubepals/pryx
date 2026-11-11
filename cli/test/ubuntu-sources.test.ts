// SPDX-License-Identifier: FSL-1.1-ALv2
/** Run the disposable Ubuntu fixture's actual source rewrite on both APT source formats.
 * Archive signatures, suites, components and explicit signing keys must survive unchanged. */
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
