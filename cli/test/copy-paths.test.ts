// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host copy paths reject existing directory symlinks before touching their targets. */
import { expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Pryx, type Sandbox } from '@cubepals/pryx'
import { download, downloadDir, upload } from '../../sdk/typescript/src/local-files.ts'
import { copy } from '../src/sandbox-data.ts'

function offlineClient(state: 'stopped' | 'sleeping' | 'resting', calls: string[], badName?: string) {
  return new Pryx({
    url: 'https://control.test',
    key: 'project-key',
    fetch: (async (input) => {
      const url = new URL(String(input)),
        path = url.searchParams.get('path')
      calls.push(url.pathname)
      if (url.pathname === '/v1/sandboxes/box') return Response.json({ id: 'box', state })
      const entry = (name: string, kind = 'directory') => ({ name, kind, size: 4, modified: 0 })
      if (url.pathname === '/v1/sandboxes/box/data/stat')
        return Response.json(entry('source', path?.endsWith('/payload') ? 'file' : 'directory'))
      if (url.pathname === '/v1/sandboxes/box/data/list')
        return Response.json({
          files:
            badName !== undefined
              ? [entry(badName)]
              : path === 'source'
                ? [entry('empty'), entry('nested')]
                : path === 'source/nested'
                  ? [entry('payload', 'file')]
                  : [],
        })
      if (url.pathname === '/v1/sandboxes/box/data/content' && path === 'source/nested/payload')
        return new Response(new Uint8Array([0, 255, 10, 42]))
      throw new Error(`Unexpected request: ${url.pathname}`)
    }) as typeof fetch,
  })
}

for (const state of ['stopped', 'sleeping', 'resting'] as const)
  test(`directory copies from ${state} data never request the agent or wake compute`, async () => {
    const root = mkdtempSync(join(tmpdir(), 'pryx-copy-offline-')),
      calls: string[] = []
    try {
      await copy(offlineClient(state, calls), 'box:source', root)
      expect(new Uint8Array(readFileSync(join(root, 'nested/payload')))).toEqual(
        new Uint8Array([0, 255, 10, 42]),
      )
      expect(existsSync(join(root, 'empty'))).toBe(true)
      expect(
        calls.every((path) => path === '/v1/sandboxes/box' || path.startsWith('/v1/sandboxes/box/data/')),
      ).toBe(true)
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  })

test('offline directory copies reject unsafe remote names and local directory links', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-copy-offline-paths-'))
  try {
    for (const name of ['', '../outside', '/absolute', 'nested/file', '..', '.', '\\escape', 'bad\0name'])
      await expect(copy(offlineClient('stopped', [], name), 'box:source', root)).rejects.toThrow(
        'Unsafe remote directory entry',
      )
    const outside = join(root, 'outside'),
      destination = join(root, 'download')
    mkdirSync(outside)
    mkdirSync(destination)
    writeFileSync(join(outside, 'payload'), 'untouched')
    symlinkSync(outside, join(destination, 'nested'))
    await expect(copy(offlineClient('stopped', []), 'box:source', destination)).rejects.toThrow(
      'Local transfers refuse symlinks',
    )
    expect(readFileSync(join(outside, 'payload'), 'utf8')).toBe('untouched')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

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
