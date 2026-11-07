// SPDX-License-Identifier: FSL-1.1-ALv2
/** A restart retains image credentials and keeps their files private. Kernel/container launch
 * is exercised by the ordinary-image acceptance, not an in-memory runtime substitute. */
import { expect, test } from 'bun:test'
import {
  chmodSync,
  closeSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { credentials, privateLog } from '../../deploy/image-state.ts'

test('image credentials persist unchanged with private modes', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-image-state-'))
  try {
    const first = await credentials(root)
    expect(await credentials(root)).toEqual(first)
    expect(first.key).toStartWith('pryx_')
    expect(first.operator).not.toBe(first.edge)
    expect(first.password).not.toBe(first.storeSecret)
    expect(statSync(join(root, 'private')).mode & 0o777).toBe(0o700)
    expect(statSync(join(root, 'private', 'credentials.json')).mode & 0o777).toBe(0o600)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('private image logs reject symlinks and public file modes', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-image-log-'))
  try {
    await credentials(root)
    const target = join(root, 'unrelated'),
      log = join(root, 'private', 'node.log')
    writeFileSync(target, 'untouched', { mode: 0o600 })
    symlinkSync(target, log)
    expect(() => privateLog(root, 'node')).toThrow()
    expect(readFileSync(target, 'utf8')).toBe('untouched')
    rmSync(log)
    writeFileSync(log, '', { mode: 0o644 })
    chmodSync(log, 0o644)
    expect(() => privateLog(root, 'node')).toThrow()
    chmodSync(log, 0o600)
    closeSync(privateLog(root, 'node'))
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
