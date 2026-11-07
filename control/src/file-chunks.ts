// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public v1 JSON file envelopes over binary agent data. SDKs use binary streams directly.
 * Temporary positional-write staging is private control storage; no sandbox shell is involved. */

import { mkdtemp, open, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { z } from 'zod'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import type { Agents } from './agents.ts'
import { SandboxError } from './sandboxes.ts'
import { SDK_DEFAULTS } from './sdk-defaults.ts'

const bytes = SDK_DEFAULTS.fileChunkBytes
export const chunkOffset = z.coerce
  .number()
  .int()
  .nonnegative()
  .max(Number.MAX_SAFE_INTEGER - bytes)
  .refine((n) => n % bytes === 0, 'Offset must be a chunk boundary')
export const chunkWrite = z
  .object({
    offset: chunkOffset,
    dataBase64: z
      .string()
      .max(Math.ceil(bytes / 3) * 4)
      .regex(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/),
  })
  .strict()
export class FileChunks {
  constructor(private agents: Agents) {}
  private async content(
    id: string,
    project: string,
    path: string,
    requestUrl: string,
    body: BodyInit | undefined,
    signal: AbortSignal,
  ) {
    if (
      !path ||
      path.startsWith('/') ||
      path.includes('\0') ||
      path.split('/').some((p) => !p || p === '.' || p === '..')
    )
      throw new SandboxError('invalid_request', 'File paths are relative without dot segments')
    const row = await this.agents.boxes.record(id, project)
    if (row.state !== 'running')
      return this.agents.data(
        id,
        project,
        'content',
        body === undefined ? 'GET' : 'PUT',
        path,
        signal,
        body === undefined ? undefined : (new Response(body).body ?? undefined),
      )
    const token = await this.agents.token(id, project, requestUrl)
    const response = await fetch(`${token.url}/_pryx/files/content?${new URLSearchParams({ path })}`, {
      method: body === undefined ? 'GET' : 'PUT',
      body,
      redirect: 'manual',
      signal,
      headers: { authorization: `Bearer ${token.token}` },
    })
    if (!response.ok) {
      await response.body?.cancel()
      throw new SandboxError('invalid_request', 'Agent file request refused')
    }
    return response
  }
  async file(
    id: string,
    project: string,
    path: string,
    data: string | undefined,
    requestUrl: string,
    signal: AbortSignal,
  ) {
    if (data !== undefined) {
      if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(data))
        throw new SandboxError('invalid_request', 'File data must be base64')
      await (
        await this.content(id, project, path, requestUrl, Buffer.from(data, 'base64'), signal)
      ).body?.cancel()
      return { written: true }
    }
    const response = await this.content(id, project, path, requestUrl, undefined, signal)
    const limit = nodeSchema['x-pryx-agent'].maxRequestBytes
    const reader = response.body?.getReader()
    const pieces: Uint8Array[] = []
    let size = 0
    try {
      while (reader) {
        const part = await reader.read()
        if (part.done) break
        size += part.value.length
        if (size > limit)
          throw new SandboxError(
            'invalid_request',
            'Use binary /data/content or file-chunks for files larger than the JSON limit',
          )
        pieces.push(part.value)
      }
    } finally {
      await reader?.cancel()
    }
    const content = Buffer.concat(pieces, size)
    return {
      archiveBase64: Buffer.from(await new Bun.Archive({ [path]: new Uint8Array(content) }).bytes()).toString(
        'base64',
      ),
    }
  }
  async transfer(
    id: string,
    project: string,
    path: string,
    offset: number,
    data: string | undefined,
    requestUrl: string,
    signal: AbortSignal,
  ) {
    if (data === undefined) {
      const response = await this.content(id, project, path, requestUrl, undefined, signal)
      const reader = response.body?.getReader()
      let skipped = 0,
        size = 0
      const chunks: Uint8Array[] = []
      try {
        while (reader && size < bytes) {
          const part = await reader.read()
          if (part.done) break
          const start = Math.max(0, offset - skipped)
          skipped += part.value.length
          const chunk = part.value.subarray(start, start + bytes - size)
          if (chunk.length) {
            chunks.push(chunk)
            size += chunk.length
          }
        }
      } finally {
        await reader?.cancel()
      }
      const content = Buffer.concat(chunks, size)
      return {
        archiveBase64: Buffer.from(await new Bun.Archive({ data: content }).bytes()).toString('base64'),
      }
    }
    const content = Buffer.from(data, 'base64')
    if (content.length > bytes)
      throw new SandboxError('invalid_request', 'File chunk exceeds its transfer bound')
    // Every chunk write to one path takes its turn, across control instances: a positional write
    // reads, patches and replaces the whole file, so two at once would lose one of them.
    await this.agents.boxes.locked(JSON.stringify(['file-chunks', id, path]), () =>
      this.write(id, project, path, offset, content, requestUrl, signal),
    )
    return { written: true }
  }
  private async write(
    id: string,
    project: string,
    path: string,
    offset: number,
    content: Buffer<ArrayBuffer>,
    requestUrl: string,
    signal: AbortSignal,
  ) {
    if (offset === 0)
      await (await this.content(id, project, path, requestUrl, content, signal)).body?.cancel()
    else {
      // v1's JSON chunk envelope requires a positional write. Spool bytes to a private control
      // temp file before replacing the agent file, keeping memory bounded and never using exec.
      const temporary = await mkdtemp(join(tmpdir(), 'pryx-file-'))
      const local = join(temporary, 'data')
      try {
        await Bun.write(local, await this.content(id, project, path, requestUrl, undefined, signal))
        const file = await open(local, 'r+')
        try {
          await file.write(content, 0, content.length, offset)
        } finally {
          await file.close()
        }
        await (
          await this.content(id, project, path, requestUrl, Bun.file(local).stream(), signal)
        ).body?.cancel()
      } finally {
        await rm(temporary, { recursive: true, force: true })
      }
    }
  }
}
