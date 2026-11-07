// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host copy paths reject existing directory symlinks before touching their targets. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Pryx, Sandbox } from '@cubepals/pryx'
import { download, downloadDir, upload } from '../../sdk/typescript/src/local-files.ts'
import { copy } from '../src/sandbox-data.ts'

test('recursive downloads cannot traverse local directory symlinks', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-copy-path-'))
  const destination = join(root, 'download'),
    outside = join(root, 'outside')
  mkdirSync(destination)
  mkdirSync(outside)
  writeFileSync(join(outside, 'file'), 'untouched')
  symlinkSync(outside, join(destination, 'link'))
  const readStream = async () =>
    new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('replacement'))
        controller.close()
      },
    })
  const disk = {
    readStream,
    stat: async () => ({ kind: 'file', size: 11 }),
    write: async () => {},
    mkdir: async () => {},
  }
  const box = {
    files: {
      stat: async (path: string) => ({ kind: path.endsWith('/file') ? 'file' : 'directory' }),
      download: (source: string, destination: string) => download(disk, source, destination),
      downloadDir: async (_source: string, destination: string) => {
        const bytes = await new Bun.Archive({ 'link/file': 'replacement' }).bytes()
        return downloadDir(new Blob([bytes]).stream(), destination)
      },
    },
  } as unknown as Sandbox
  const client = { sandbox: async () => box } as unknown as Pryx
  try {
    await expect(copy(client, 'box:source', destination)).rejects.toThrow('Local transfers refuse symlinks')
    expect(readFileSync(join(outside, 'file'), 'utf8')).toBe('untouched')
    await expect(copy(client, 'box:source/file', join(destination, 'link/file'))).rejects.toThrow(
      'Local transfers refuse symlinks',
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
        files: {
          upload: (source: string, destination: string) =>
            upload(
              {
                write: async () => {
                  written = true
                },
                readStream: async () => new ReadableStream(),
                stat: async () => ({ kind: 'file', size: 7 }),
                mkdir: async () => {},
              },
              source,
              destination,
            ),
        },
      }),
    } as unknown as Pryx
    await expect(copy(client, join(root, 'link/file'), 'box:file')).rejects.toThrow(
      'Local transfers refuse symlinks',
    )
    expect(written).toBe(false)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
