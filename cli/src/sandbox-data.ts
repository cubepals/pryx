// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Interactive terminals and streamed recursive copies use the public SDK. Local symlinks are
 * refused, and remote directory entries must be single names before joining local paths. */
import { createReadStream, createWriteStream } from 'node:fs'
import { lstat, mkdir, readdir } from 'node:fs/promises'
import { basename, dirname, join } from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import type { Pryx, Sandbox } from '@cubepals/pryx'

export async function terminal(box: Sandbox) {
  const term = await box.terminal({ cols: process.stdout.columns ?? 80, rows: process.stdout.rows ?? 24 })
  const raw = process.stdin.isRaw
  const send = (data: Buffer) => term.stdin.write(data)
  const resize = () => term.resize(process.stdout.columns ?? 80, process.stdout.rows ?? 24)
  if (process.stdin.isTTY) process.stdin.setRawMode(true)
  process.stdin.on('data', send)
  process.stdout.on('resize', resize)
  process.stdin.resume()
  try {
    for await (const chunk of term.stream()) {
      if (!process.stdout.write(chunk.data))
        await new Promise<void>((resolve) => process.stdout.once('drain', resolve))
    }
  } finally {
    term.close()
    process.stdin.off('data', send)
    process.stdout.off('resize', resize)
    if (process.stdin.isTTY) process.stdin.setRawMode(raw)
    process.stdin.pause()
  }
}

export async function copy(client: Pryx, source: string, destination: string) {
  const remote = /^([^/:]+):(.*)$/
  const from = remote.exec(source),
    to = remote.exec(destination)
  if (Boolean(from) === Boolean(to)) throw new Error('cp needs one sandbox:path and one local path')
  const match = (from ?? to) as RegExpExecArray
  const box = await client.sandbox(match[1] as string)
  const path = match[2] || '.'
  if (from) await download(box, path, destination)
  else await upload(box, source, path)
}

async function upload(box: Sandbox, source: string, destination: string): Promise<void> {
  const info = await lstat(source)
  if (info.isSymbolicLink()) throw new Error('cp refuses local symlinks')
  if (info.isDirectory()) {
    await box.files.mkdir(destination)
    for (const entry of await readdir(source))
      await upload(box, join(source, entry), `${destination}/${entry}`)
  } else if (info.isFile()) await box.write(destination, createReadStream(source))
  else throw new Error('cp accepts regular files and directories')
}

async function download(box: Sandbox, source: string, destination: string): Promise<void> {
  const info = await box.files.stat(source)
  if (info.kind === 'directory') {
    await mkdir(destination, { recursive: true })
    for (const entry of await box.files.list(source)) {
      if (basename(entry.name) !== entry.name || ['.', '..'].includes(entry.name))
        throw new Error('Invalid remote filename')
      await download(box, `${source}/${entry.name}`, join(destination, entry.name))
    }
  } else if (info.kind === 'file') {
    await mkdir(dirname(destination), { recursive: true })
    const existing = await lstat(destination).catch((error: NodeJS.ErrnoException) => {
      if (error.code !== 'ENOENT') throw error
      return null
    })
    if (existing?.isSymbolicLink()) throw new Error('cp refuses local symlinks')
    await pipeline(
      Readable.from(await box.readStream(source)),
      createWriteStream(destination, { flags: 'w' }),
    )
  } else throw new Error('cp accepts regular files and directories')
}
