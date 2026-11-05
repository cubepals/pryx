// SPDX-License-Identifier: FSL-1.1-ALv2
/** Failure diagnostics redact persisted secrets and refuse private-directory symlinks.
 * The service journal is an owned fixture; no machine service or real credential is read. */
import { expect, test } from 'bun:test'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

test('image diagnostics redact credentials and refuse an unsafe private directory', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-diagnostics-'))
  try {
    const privateDir = join(directory, 'private'),
      tools = join(directory, 'bin')
    mkdirSync(privateDir, { mode: 0o700 })
    mkdirSync(tools)
    const secret = 'diagnostic-test-secret-value'
    writeFileSync(join(privateDir, 'credentials.json'), JSON.stringify({ key: secret }), { mode: 0o600 })
    writeFileSync(join(privateDir, 'node.log'), `node: ${secret}\n`, { mode: 0o600 })
    const journal = join(directory, 'journal')
    writeFileSync(journal, `journal: ${secret}\n`)
    writeFileSync(join(tools, 'journalctl'), '#!/bin/sh\ncat "$PRYX_TEST_JOURNAL"\n', { mode: 0o700 })
    async function run() {
      const child = Bun.spawn(
        [Bun.which('python3') ?? 'python3', 'control/tools/image-diagnostics.py', directory],
        {
          env: { ...process.env, PATH: `${tools}:${process.env.PATH}`, PRYX_TEST_JOURNAL: journal },
          stdout: 'pipe',
          stderr: 'pipe',
        },
      )
      const [output, error, code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ])
      expect({ error, code }).toEqual({ error: '', code: 0 })
      return output
    }
    const output = await run()
    expect(output).toContain('journal: [redacted]')
    expect(output).toContain('node: [redacted]')
    expect(output).not.toContain(secret)
    expect(readFileSync(join(privateDir, 'node.log'), 'utf8')).toContain(secret)
    rmSync(privateDir, { recursive: true })
    const target = join(directory, 'unrelated')
    mkdirSync(target, { mode: 0o700 })
    writeFileSync(join(target, 'node.log'), 'unrelated-private-marker', { mode: 0o600 })
    symlinkSync(target, privateDir)
    expect(await run()).not.toContain('unrelated-private-marker')
    rmSync(privateDir)
    mkdirSync(privateDir, { mode: 0o700 })
    chmodSync(privateDir, 0o755)
    writeFileSync(join(privateDir, 'node.log'), 'public-directory-marker', { mode: 0o600 })
    expect(await run()).not.toContain('public-directory-marker')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
