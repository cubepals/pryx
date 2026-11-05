// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host copy paths reject existing directory symlinks before touching their targets. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Pryx, Sandbox } from '@cubepals/pryx'
import { copy } from '../src/sandbox-data.ts'

test('recursive downloads cannot traverse local directory symlinks', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-copy-path-'))
  const destination = join(root, 'download'),
    outside = join(root, 'outside')
  mkdirSync(destination)
  mkdirSync(outside)
  writeFileSync(join(outside, 'file'), 'untouched')
  symlinkSync(outside, join(destination, 'link'))
  const box = {
    files: {
      stat: async (path: string) => ({ kind: path.endsWith('/file') ? 'file' : 'directory' }),
      list: async (path: string) => (path === 'source' ? [{ name: 'link' }] : [{ name: 'file' }]),
    },
    readStream: async () =>
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode('replacement'))
          controller.close()
        },
      }),
  } as unknown as Sandbox
  const client = { sandbox: async () => box } as unknown as Pryx
  try {
    await expect(copy(client, 'box:source', destination)).rejects.toThrow('cp refuses local symlinks')
    expect(readFileSync(join(outside, 'file'), 'utf8')).toBe('untouched')
    await expect(copy(client, 'box:source/file', join(destination, 'link/file'))).rejects.toThrow(
      'cp refuses local symlinks',
    )
    expect(readFileSync(join(outside, 'file'), 'utf8')).toBe('untouched')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
test('uploads reject symlink components above the selected file', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-copy-upload-'))
  try {
    const directory = join(root, 'directory')
    mkdirSync(directory)
    writeFileSync(join(directory, 'file'), 'private')
    symlinkSync(directory, join(root, 'link'))
    let written = false
    const client = {
      sandbox: async () => ({
        write: async () => {
          written = true
        },
      }),
    } as unknown as Pryx
    await expect(copy(client, join(root, 'link/file'), 'box:file')).rejects.toThrow(
      'cp refuses local symlinks',
    )
    expect(written).toBe(false)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
