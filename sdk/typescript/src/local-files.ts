// SPDX-License-Identifier: FSL-1.1-ALv2
/** Local disk transfers stream bounded chunks and tar entries. Remote names cannot traverse
 * local directories or create links. These Node APIs are imported only for disk operations. */

import { createHash } from 'node:crypto'
import { constants } from 'node:fs'
import { lstat, mkdir, open, readdir } from 'node:fs/promises'
import { dirname, join, parse, resolve, sep } from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { StatusCodes } from 'http-status-codes'
import { extract, pack } from 'tar-stream'
import type { components } from './generated/api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
export type Progress = (bytes: number, total?: number) => void
export type DiskFiles = {
  write(path: string, data: AsyncIterable<Uint8Array>): Promise<void>
  readStream(path: string): Promise<ReadableStream<Uint8Array>>
  stat(path: string): Promise<{ kind: string; size: number }>
  mkdir(path: string): Promise<unknown>
}
const fdRoot =
  process.platform === 'linux' ? '/proc/self/fd' : process.platform === 'darwin' ? '/dev/fd' : undefined
function descriptorPath(fd: number, name = '') {
  if (!fdRoot) throw new Error('Safe local transfers require descriptor paths')
  return fdRoot + '/' + fd + (name ? '/' + name : '')
}
async function directory(path: string, create = false) {
  path = resolve(path)
  let file = await open(parse(path).root, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
  try {
    for (const part of path.slice(parse(path).root.length).split(sep).filter(Boolean)) {
      const child = descriptorPath(file.fd, part)
      if (create)
        await mkdir(child).catch((error: NodeJS.ErrnoException) => {
          if (error.code !== 'EEXIST') throw error
        })
      const next = await open(child, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
      await file.close()
      file = next
    }
    return file
  } catch (error) {
    await file.close()
    if (error instanceof Error && 'code' in error && ['ELOOP', 'ENOTDIR'].includes(String(error.code)))
      throw new Error('Local transfers refuse symlinks and non-directory parents')
    throw error
  }
}
export async function localPath(path: string) {
  return resolve(path)
}
async function input(path: string) {
  path = resolve(path)
  const parent = await directory(dirname(path))
  try {
    const file = await open(
      descriptorPath(parent.fd, parse(path).base),
      constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
    )
    if (!(await file.stat()).isFile()) {
      await file.close()
      throw new Error('Expected a regular file')
    }
    return file
  } finally {
    await parent.close()
  }
}
async function output(path: string) {
  path = resolve(path)
  const parent = await directory(dirname(path), true)
  try {
    const file = await open(
      descriptorPath(parent.fd, parse(path).base),
      constants.O_WRONLY | constants.O_CREAT | constants.O_NOFOLLOW | constants.O_NONBLOCK,
      0o600,
    )
    if (!(await file.stat()).isFile()) {
      await file.close()
      throw new Error('Expected a regular file')
    }
    await file.truncate(0)
    return file
  } finally {
    await parent.close()
  }
}
export async function upload(files: DiskFiles, local: string, remote: string, onProgress?: Progress) {
  const file = await input(local)
  const total = (await file.stat()).size
  let bytes = 0
  const stream = file.createReadStream()
  async function* chunks() {
    for await (const chunk of stream) {
      const data = chunk as Buffer
      bytes += data.length
      onProgress?.(bytes, total)
      yield data
    }
  }
  try {
    await files.write(remote, chunks())
  } finally {
    stream.destroy()
    await file.close()
  }
}
export async function download(
  files: Pick<DiskFiles, 'stat' | 'readStream'>,
  remote: string,
  local: string,
  onProgress?: Progress,
) {
  const info = await files.stat(remote)
  const file = await output(local)
  let bytes = 0
  const source = Readable.fromWeb(
    (await files.readStream(remote)) as unknown as import('node:stream/web').ReadableStream,
  )
  try {
    for await (const chunk of source) {
      const part = chunk as Buffer
      await file.writeFile(part)
      bytes += part.length
      onProgress?.(bytes, info.size)
    }
  } finally {
    source.destroy()
    await file.close()
  }
}
/** Stopped data has list/read access without starting an agent. Keep the same local path checks. */
export async function downloadTree(
  files: Pick<DiskFiles, 'stat' | 'readStream'> & {
    list(path: string): Promise<components['schemas']['FileInfo'][]>
  },
  remote: string,
  local: string,
  onProgress?: Progress,
) {
  let count = 0,
    bytes = 0
  async function visit(source: string, destination: string) {
    const held = await directory(destination, true)
    await held.close()
    for (const entry of await files.list(source)) {
      if (++count > SDK_DEFAULTS.archiveEntries) throw new Error('Archive exceeds entry budget')
      if (!entry.name || ['.', '..'].includes(entry.name) || /[/\\\0]/.test(entry.name))
        throw new Error('Unsafe remote directory entry')
      const path = `${source}/${entry.name}`,
        target = join(destination, entry.name)
      if (entry.kind === 'directory') await visit(path, target)
      else if (entry.kind === 'file') {
        let previous = 0
        await download(files, path, target, (current) => {
          bytes += current - previous
          previous = current
          if (bytes > SDK_DEFAULTS.archiveBytes) throw new Error('Archive exceeds byte budget')
          onProgress?.(bytes)
        })
      } else throw new Error('Directory transfers refuse links and special files')
    }
  }
  await visit(remote, local)
}
async function* entries(
  root: string,
  prefix = '',
): AsyncGenerator<{ path: string; local: string; directory: boolean; size: number; mode: number }> {
  const held = await directory(join(root, prefix))
  try {
    for (const name of await readdir(descriptorPath(held.fd))) {
      const path = prefix ? `${prefix}/${name}` : name
      const local = join(root, path)
      const info = await lstat(descriptorPath(held.fd, name))
      if (!info.isDirectory() && !info.isFile())
        throw new Error('Directory transfers refuse links and special files')
      yield { path, local, directory: info.isDirectory(), size: info.size, mode: info.mode & 0o777 }
      if (info.isDirectory()) yield* entries(root, path)
    }
  } finally {
    await held.close()
  }
}
export async function uploadDir(
  local: string,
  send: (stream: AsyncIterable<Uint8Array>) => Promise<unknown>,
  onProgress?: Progress,
) {
  local = await localPath(local)
  const archive = pack()
  let bytes = 0
  const produce = (async () => {
    try {
      for await (const entry of entries(local)) {
        const stream = archive.entry({
          name: entry.path,
          type: entry.directory ? 'directory' : 'file',
          size: entry.directory ? 0 : entry.size,
          mode: entry.mode,
        })
        if (entry.directory) stream.end(undefined)
        else {
          const file = await input(entry.local)
          try {
            await pipeline(file.createReadStream(), stream)
          } finally {
            await file.close()
          }
        }
      }
      archive.finalize()
    } catch (error) {
      archive.destroy(error as Error)
      throw error
    }
  })()
  void produce.catch(() => {})
  async function* chunks() {
    for await (const data of archive) {
      const chunk = data as Buffer
      bytes += chunk.length
      onProgress?.(bytes)
      yield chunk
    }
  }
  try {
    await send(chunks())
    await produce
  } finally {
    archive.destroy(undefined)
  }
}
export async function downloadDir(stream: ReadableStream<Uint8Array>, local: string, onProgress?: Progress) {
  local = await localPath(local)
  const root = await directory(local, true)
  await root.close()
  const archive = extract()
  let bytes = 0
  const source = Readable.fromWeb(stream as unknown as import('node:stream/web').ReadableStream)
  source.on('data', (chunk: Buffer) => {
    bytes += chunk.length
    if (bytes > SDK_DEFAULTS.archiveBytes) {
      source.destroy(new Error('Archive exceeds byte budget'))
      return
    }
    try {
      onProgress?.(bytes)
    } catch (error) {
      source.destroy(error as Error)
    }
  })
  const pump = pipeline(source, archive)
  void pump.catch(() => {})
  let count = 0
  const modes: { path: string; mode: number }[] = []
  try {
    for await (const entry of archive) {
      if (++count > SDK_DEFAULTS.archiveEntries) throw new Error('Archive exceeds entry budget')
      const { name, type } = entry.header
      if (
        name.startsWith('/') ||
        name.includes('\\') ||
        name.split('/').some((part) => part === '..') ||
        name.includes('\0')
      )
        throw new Error('Unsafe archive path')
      const path = await localPath(join(local, name))
      if (type === 'directory') {
        const held = await directory(path, true)
        await held.close()
        modes.push({ path, mode: (entry.header.mode ?? 0o755) & 0o777 })
        entry.resume()
      } else if (type === 'file') {
        const file = await output(path)
        try {
          for await (const chunk of entry) {
            if (!(chunk instanceof Uint8Array)) throw new Error('Invalid archive data')
            await file.writeFile(chunk)
          }
          await file.chmod((entry.header.mode ?? 0o600) & 0o777)
        } finally {
          await file.close()
        }
      } else throw new Error('Archive links and special files are refused')
    }
    await pump
    for (const { path, mode } of modes.reverse()) {
      const held = await directory(path)
      try {
        await held.chmod(mode)
      } finally {
        await held.close()
      }
    }
  } finally {
    source.destroy()
    archive.destroy(undefined)
  }
}
async function digest(stream: AsyncIterable<Uint8Array>) {
  const hash = createHash('sha256')
  for await (const part of stream) hash.update(part)
  return hash.digest('hex')
}
export async function sync(files: DiskFiles, local: string, remote: string) {
  local = await localPath(local)
  await files.mkdir(remote)
  const uploaded: string[] = [],
    unchanged: string[] = []
  for await (const entry of entries(local)) {
    const path = `${remote}/${entry.path}`
    if (entry.directory) {
      await files.mkdir(path)
      continue
    }
    const file = await input(entry.local)
    let hash: string
    try {
      hash = await digest(file.createReadStream())
    } finally {
      await file.close()
    }
    let same = false
    try {
      same =
        hash ===
        (await digest(
          Readable.fromWeb(
            (await files.readStream(path)) as unknown as import('node:stream/web').ReadableStream,
          ),
        ))
    } catch (error) {
      if (!(error instanceof Error && 'status' in error && error.status === StatusCodes.NOT_FOUND))
        throw error
    }
    if (same) unchanged.push(entry.path)
    else {
      await upload(files, entry.local, path)
      uploaded.push(entry.path)
    }
  }
  return { uploaded, unchanged }
}
