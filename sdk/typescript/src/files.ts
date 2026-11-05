// SPDX-License-Identifier: FSL-1.1-ALv2
/** Bounded file chunks over the public exec/tar transport. Archives are read in memory, never extracted on the client filesystem. */

import { type Agent, binaryBody } from './agent.ts'
import type { Sandbox } from './api.ts'
import { type Pryx, PryxError } from './api.ts'
import type { components } from './generated/api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
export type FileData = string | Uint8Array | ReadableStream<Uint8Array> | AsyncIterable<Uint8Array>
type FileInfo = components['schemas']['FileInfo']

function dataStream(data: FileData): ReadableStream<Uint8Array> {
  if (typeof data === 'string' || data instanceof Uint8Array) {
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
    return new ReadableStream({
      start(controller) {
        controller.enqueue(bytes)
        controller.close()
      },
    })
  }
  if ('getReader' in data) return data
  const iterator = data[Symbol.asyncIterator]()
  return new ReadableStream({
    pull: async (controller) => {
      const part = await iterator.next()
      if (part.done) controller.close()
      else controller.enqueue(part.value)
    },
    cancel: async () => {
      await iterator.return?.()
    },
  })
}

function archiveData(encoded: string): Uint8Array {
  const tar = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0))
  const text = (start: number, length: number) =>
    new TextDecoder()
      .decode(tar.subarray(start, start + length))
      .replace(/\0.*$/s, '')
      .trim()
  const size = Number.parseInt(text(124, 12), 8)
  const checksum = Number.parseInt(text(148, 8), 8)
  let sum = 0
  for (let index = 0; index < 512; index++) sum += index >= 148 && index < 156 ? 32 : (tar[index] ?? 0)
  if (
    text(0, 100) !== 'data' ||
    !['', '0'].includes(text(156, 1)) ||
    !Number.isSafeInteger(size) ||
    size < 0 ||
    size > SDK_DEFAULTS.fileChunkBytes ||
    tar.length < 512 + size ||
    sum !== checksum
  )
    throw new PryxError(0, 'invalid_archive', 'Invalid file chunk archive')
  return tar.slice(512, 512 + size)
}
export class Files {
  constructor(
    private api: Pryx,
    private id: string,
    private direct?: { agent: Agent; view: () => Sandbox },
  ) {}
  private path(path: string) {
    return `/v1/sandboxes/${encodeURIComponent(this.id)}/file-chunks/${path.split('/').map(encodeURIComponent).join('/')}`
  }
  async write(path: string, data: FileData): Promise<void> {
    if (this.direct?.view().agent && this.direct.view().state === 'running') {
      await this.direct.agent.raw(
        `/_pryx/files/content?${new URLSearchParams({ path })}`,
        'PUT',
        typeof data === 'string' || data instanceof Uint8Array ? binaryBody(data) : dataStream(data),
      )
      return
    }
    if (this.direct?.view().files) {
      await this.api.transfer(
        this.nodePath('content', path),
        'PUT',
        typeof data === 'string' || data instanceof Uint8Array ? binaryBody(data) : dataStream(data),
      )
      return
    }
    if (typeof data !== 'string' && !(data instanceof Uint8Array)) {
      let pending = new Uint8Array(SDK_DEFAULTS.fileChunkBytes),
        held = 0,
        offset = 0
      const flush = async () => {
        await this.chunk(path, offset, pending.subarray(0, held))
        offset += held
        pending = new Uint8Array(SDK_DEFAULTS.fileChunkBytes)
        held = 0
      }
      for await (const bytes of dataStream(data)) {
        for (let at = 0; at < bytes.length; ) {
          const n = Math.min(bytes.length - at, pending.length - held)
          pending.set(bytes.subarray(at, at + n), held)
          held += n
          at += n
          if (held === pending.length) await flush()
        }
      }
      if (held || offset === 0) await flush()
      return
    }
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
    for (let offset = 0; offset < bytes.length || offset === 0; offset += SDK_DEFAULTS.fileChunkBytes) {
      const chunk = bytes.subarray(offset, offset + SDK_DEFAULTS.fileChunkBytes)
      await this.chunk(path, offset, chunk)
    }
  }
  async readBytes(path: string): Promise<Uint8Array> {
    if ((this.direct?.view().agent && this.direct.view().state === 'running') || this.direct?.view().files)
      return new Uint8Array(await new Response(await this.readStream(path)).arrayBuffer())
    const chunks: Uint8Array[] = []
    let offset = 0
    while (true) {
      const response = await this.api.request<{ archiveBase64: string }>(
        `${this.path(path)}?offset=${offset}`,
      )
      const chunk = archiveData(response.archiveBase64)
      chunks.push(chunk)
      offset += chunk.length
      if (chunk.length < SDK_DEFAULTS.fileChunkBytes) break
    }
    const result = new Uint8Array(offset)
    let at = 0
    for (const chunk of chunks) {
      result.set(chunk, at)
      at += chunk.length
    }
    return result
  }
  async read(path: string) {
    return new TextDecoder().decode(await this.readBytes(path))
  }
  private nodePath(operation: string, path: string) {
    return `/v1/sandboxes/${encodeURIComponent(this.id)}/data/${operation}?${new URLSearchParams({ path })}`
  }
  private chunk(path: string, offset: number, bytes: Uint8Array) {
    return this.api.request(this.path(path), 'PUT', {
      offset,
      dataBase64: btoa(Array.from(bytes, (byte) => String.fromCharCode(byte)).join('')),
    })
  }
  async readStream(path: string): Promise<ReadableStream<Uint8Array>> {
    if (this.direct?.view().agent && this.direct.view().state === 'running') {
      const response = await this.direct.agent.raw(`/_pryx/files/content?${new URLSearchParams({ path })}`)
      if (!response.body) throw new PryxError(0, 'file_interrupted', 'File response has no body')
      return response.body
    }
    if (this.direct?.view().files) {
      const response = await this.api.transfer(this.nodePath('content', path))
      if (!response.body) throw new PryxError(0, 'file_interrupted', 'File response has no body')
      return response.body
    }
    let offset = 0
    return new ReadableStream({
      pull: async (controller) => {
        try {
          const response = await this.api.request<{ archiveBase64: string }>(
            `${this.path(path)}?offset=${offset}`,
          )
          const bytes = archiveData(response.archiveBase64)
          offset += bytes.length
          if (bytes.length) controller.enqueue(bytes)
          if (bytes.length < SDK_DEFAULTS.fileChunkBytes) controller.close()
        } catch (error) {
          controller.error(error)
        }
      },
    })
  }
  async list(path = '.') {
    if (this.direct?.view().agent && this.direct.view().state === 'running')
      return (
        await this.direct.agent.request<{ files: FileInfo[] }>(
          `/_pryx/files/list?${new URLSearchParams({ path })}`,
        )
      ).files
    return ((await (await this.api.transfer(this.nodePath('list', path))).json()) as { files: FileInfo[] })
      .files
  }
  async stat(path: string): Promise<FileInfo> {
    if (this.direct?.view().agent && this.direct.view().state === 'running')
      return this.direct.agent.request(`/_pryx/files/stat?${new URLSearchParams({ path })}`)
    return (await this.api.transfer(this.nodePath('stat', path))).json() as Promise<FileInfo>
  }
  mkdir(path: string) {
    return this.needAgent().request<void>('/_pryx/files/mkdir', 'POST', { path })
  }
  move(from: string, to: string) {
    return this.needAgent().request<void>('/_pryx/files/move', 'POST', { from, to })
  }
  delete(path: string) {
    return this.needAgent().request<void>(`/_pryx/files/content?${new URLSearchParams({ path })}`, 'DELETE')
  }
  upload(path: string, url: string) {
    return this.needAgent().request<void>('/_pryx/files/upload', 'POST', { path, url })
  }
  download(path: string, url: string) {
    return this.needAgent().request<void>('/_pryx/files/download', 'POST', { path, url })
  }
  async url(path: string, method: 'GET' | 'PUT' = 'GET') {
    const agent = this.needAgent()
    const grant = await agent.request<{ url: string; expiresAt: number }>('/_pryx/files/token', 'POST', {
      path,
      method,
    })
    return { ...grant, url: `${(await agent.token()).url.replace(/\/$/, '')}${grant.url}` }
  }
  private needAgent() {
    if (!this.direct?.view().agent) throw new PryxError(0, 'unsupported', 'This sandbox has no agent')
    return this.direct.agent
  }
  async *watch(path: string) {
    const pending: unknown[] = []
    let ended = false,
      wake: (() => void) | undefined,
      failure: Error | undefined
    const socket = await this.needAgent().socket(
      `/_pryx/files/watch?${new URLSearchParams({ path })}`,
      (socket) => {
        socket.addEventListener('message', (event) => {
          try {
            pending.push(JSON.parse(String(event.data)))
          } catch {
            failure = new PryxError(0, 'agent_protocol', 'Invalid file watch frame')
            socket.close()
            ended = true
          }
          if (pending.length > 128) {
            failure = new Error('File watch overflow')
            socket.close()
            ended = true
          }
          wake?.()
        })
        socket.addEventListener('close', () => {
          ended = true
          wake?.()
        })
      },
    )
    try {
      while (!ended || pending.length) {
        if (pending.length) yield pending.shift()
        else
          await new Promise<void>((resolve) => {
            wake = resolve
          })
      }
      if (failure) throw failure
    } finally {
      socket.close()
    }
  }
}
