// SPDX-License-Identifier: FSL-1.1-ALv2
/** Binary file operations use the sandbox agent while running and persistent node data while compute is stopped. */

import { StatusCodes } from 'http-status-codes'
import { type Agent, binaryBody } from './agent.ts'
import type { components } from './generated/api.ts'
import type { Progress } from './local-files.ts'
import type { Sandbox } from './transport.ts'
import { PryxError, type Transport } from './transport.ts'
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

export class Files {
  #api: Transport
  #id: string
  #direct: { agent: Agent; view: () => Sandbox }
  constructor(api: Transport, id: string, direct: { agent: Agent; view: () => Sandbox }) {
    this.#api = api
    this.#id = id
    this.#direct = direct
  }
  async write(path: string, data: FileData): Promise<void> {
    if (this.#direct.view().state === 'running') {
      await this.#direct.agent.raw(
        `/_pryx/files/content?${new URLSearchParams({ path })}`,
        'PUT',
        typeof data === 'string' || data instanceof Uint8Array ? binaryBody(data) : dataStream(data),
      )
      return
    }
    await this.#api.transfer(
      this.nodePath('content', path),
      'PUT',
      typeof data === 'string' || data instanceof Uint8Array ? binaryBody(data) : dataStream(data),
    )
    return
  }
  async readBytes(path: string): Promise<Uint8Array> {
    return new Uint8Array(await new Response(await this.readStream(path)).arrayBuffer())
  }
  async read(path: string) {
    return new TextDecoder().decode(await this.readBytes(path))
  }
  private nodePath(operation: string, path: string) {
    return `/v1/sandboxes/${encodeURIComponent(this.#id)}/data/${operation}?${new URLSearchParams({ path })}`
  }
  async readStream(path: string): Promise<ReadableStream<Uint8Array>> {
    if (this.#direct.view().state === 'running') {
      const response = await this.#direct.agent.raw(`/_pryx/files/content?${new URLSearchParams({ path })}`)
      if (!response.body) throw new PryxError(0, 'file_interrupted', 'File response has no body')
      return response.body
    }
    {
      const response = await this.#api.transfer(this.nodePath('content', path))
      if (!response.body) throw new PryxError(0, 'file_interrupted', 'File response has no body')
      return response.body
    }
  }
  async list(path = '.') {
    if (this.#direct.view().state === 'running')
      return (
        await this.#direct.agent.request<{ files: FileInfo[] }>(
          `/_pryx/files/list?${new URLSearchParams({ path })}`,
        )
      ).files
    return ((await (await this.#api.transfer(this.nodePath('list', path))).json()) as { files: FileInfo[] })
      .files
  }
  async stat(path: string): Promise<FileInfo> {
    if (this.#direct.view().state === 'running')
      return this.#direct.agent.request(`/_pryx/files/stat?${new URLSearchParams({ path })}`)
    return (await this.#api.transfer(this.nodePath('stat', path))).json() as Promise<FileInfo>
  }
  mkdir(path: string) {
    return this.#direct.agent.request<void>('/_pryx/files/mkdir', 'POST', { path })
  }
  move(from: string, to: string) {
    return this.#direct.agent.request<void>('/_pryx/files/move', 'POST', { from, to })
  }
  delete(path: string) {
    return this.#direct.agent.request<void>(`/_pryx/files/content?${new URLSearchParams({ path })}`, 'DELETE')
  }
  async exists(path: string) {
    try {
      await this.stat(path)
      return true
    } catch (error) {
      if (error instanceof PryxError && error.status === StatusCodes.NOT_FOUND) return false
      throw error
    }
  }
  async find(pattern: string) {
    return (await this.#direct.agent.requestV2<{ paths: string[] }>('/_pryx/files/find', 'POST', { pattern }))
      .paths
  }
  async grep(pattern: string, options: { paths: string[] }) {
    return (
      await this.#direct.agent.requestV2<{ matches: { path: string; line: number; text: string }[] }>(
        '/_pryx/files/grep',
        'POST',
        { pattern, ...options },
      )
    ).matches
  }
  replace(pattern: string, text: string, options: { paths: string[] }) {
    return this.#direct.agent.requestV2<{ replacements: number }>('/_pryx/files/replace', 'POST', {
      pattern,
      text,
      ...options,
    })
  }
  chmod(path: string, mode: number) {
    return this.#direct.agent.requestV2<void>('/_pryx/files/chmod', 'POST', { path, mode })
  }
  async upload(localPath: string, remote: string, options: { onProgress?: Progress } = {}) {
    return (await import('./local-files.ts')).upload(this, localPath, remote, options.onProgress)
  }
  async download(remote: string, localPath: string, options: { onProgress?: Progress } = {}) {
    return (await import('./local-files.ts')).download(this, remote, localPath, options.onProgress)
  }
  async uploadDir(localDir: string, remote: string, options: { onProgress?: Progress } = {}) {
    await this.#direct.agent.requireVersion()
    return (await import('./local-files.ts')).uploadDir(
      localDir,
      async (stream) =>
        this.#direct.agent.raw(
          `/_pryx/files/archive?${new URLSearchParams({ path: remote })}`,
          'PUT',
          dataStream(stream),
        ),
      options.onProgress,
    )
  }
  async downloadDir(remote: string, localDir: string, options: { onProgress?: Progress } = {}) {
    if (this.#direct.view().state !== 'running')
      return (await import('./local-files.ts')).downloadTree(this, remote, localDir, options.onProgress)
    await this.#direct.agent.requireVersion()
    const response = await this.#direct.agent.raw(
      `/_pryx/files/archive?${new URLSearchParams({ path: remote })}`,
    )
    if (!response.body) throw new PryxError(0, 'file_interrupted', 'Archive has no body')
    return (await import('./local-files.ts')).downloadDir(response.body, localDir, options.onProgress)
  }
  async sync(localDir: string, remoteDir: string) {
    return (await import('./local-files.ts')).sync(this, localDir, remoteDir)
  }
  async url(path: string, method: 'GET' | 'PUT' = 'GET') {
    const agent = this.#direct.agent
    const grant = await agent.request<{ url: string; expiresAt: number }>('/_pryx/files/token', 'POST', {
      path,
      method,
    })
    return { ...grant, url: `${(await agent.token()).url.replace(/\/$/, '')}${grant.url}` }
  }
  async *watch(path: string) {
    const pending: unknown[] = []
    let ended = false,
      wake: (() => void) | undefined,
      failure: Error | undefined
    const socket = await this.#direct.agent.socket(
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
