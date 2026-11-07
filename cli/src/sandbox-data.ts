// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Interactive terminals and streamed recursive copies use the public SDK. Local symlinks are
 * refused, and remote directory entries must be single names before joining local paths. */
import { lstat } from 'node:fs/promises'
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
  if (from)
    await ((await box.files.stat(path)).kind === 'directory'
      ? box.files.downloadDir(path, destination)
      : box.files.download(path, destination))
  else
    await ((await lstat(source)).isDirectory()
      ? box.files.uploadDir(source, path)
      : box.files.upload(source, path))
}
