// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real interprocess SQLite locking survives a killed owner and refuses filesystem aliases.
 * It serializes lifecycle mutations; it is not the lifetime lock of a running supervisor. */
import { expect, test } from 'bun:test'
import { linkSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { SECOND_MS } from '../../control/src/durations.ts'
import { acquireLifecycle } from '../src/up-local-lock.ts'

test('a real competing process is refused and a killed owner releases the lock', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-lock-'))
  const path = join(directory, 'operation.sqlite')
  const module = new URL('../src/up-local-lock.ts', import.meta.url).pathname
  const script = `import {acquireLifecycle} from ${JSON.stringify(module)}; acquireLifecycle(${JSON.stringify(path)}); Bun.gc(true); console.log('held'); setInterval(() => {}, ${SECOND_MS});`
  const child = Bun.spawn([process.execPath, '-e', script], {
    stdin: 'ignore',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  try {
    const reader = child.stdout.getReader()
    const first = await reader.read()
    if (first.done) throw new Error(await new Response(child.stderr).text())
    expect(new TextDecoder().decode(first.value)).toContain('held')
    expect(() => acquireLifecycle(path)).toThrow('Another pryx lifecycle operation')
    child.kill('SIGKILL')
    await child.exited
    const release = acquireLifecycle(path)
    expect(() => acquireLifecycle(path)).toThrow('Another pryx lifecycle operation')
    release()
    release()
    acquireLifecycle(path)()
  } finally {
    if (child.exitCode === null) child.kill('SIGKILL')
    await child.exited
    rmSync(directory, { recursive: true, force: true })
  }
})
test('lock aliases cannot open another file', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-lock-link-'))
  try {
    const original = join(directory, 'original.sqlite')
    writeFileSync(original, '', { mode: 0o600 })
    const link = join(directory, 'symbolic.sqlite'),
      hard = join(directory, 'hard.sqlite')
    symlinkSync(original, link)
    expect(() => acquireLifecycle(link)).toThrow()
    linkSync(original, hard)
    expect(() => acquireLifecycle(hard)).toThrow('one link')
    expect(readFileSync(original).byteLength).toBe(0)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
