// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** An upload in parts this store began: its parts by number, until it completes or is dropped. */

import { createHash, randomUUID } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { type ArchiveStore, ArtifactMismatch } from '../src/archive-store.ts'
import { MAX_PUT_BYTES, partPlan } from '../src/multipart.ts'
import type {
  ArchiveTarget,
  DownloadTarget,
  PartsUpload,
  PutPart,
  UploadTarget,
} from '../src/runtime-model.ts'

interface Upload {
  key: string
  parts: Map<number, Buffer>
}

const etagOf = (bytes: Buffer) => `"${createHash('md5').update(bytes).digest('hex')}"`

/**
 * An object store in memory, for tests that need the archives capability without S3: it keeps
 * what is put in it by key, checks bytes against the sha512 they are stored under as the real
 * store does, and serves its "presigned" links from a local listener, which counts downloads.
 * Uploads in parts work as S3's do: each part is PUT to its own link, and nothing is stored until
 * the upload completes with the parts' ETags.
 */
export class MemoryStore implements ArchiveStore {
  readonly objects = new Map<string, Buffer>()
  readonly uploads = new Map<string, Upload>()
  downloads = 0
  /** Uploads in parts dropped, and completed: what a test checks was cleaned up. */
  aborted = 0
  completed = 0
  /** The most one PUT carries here: lowered, a test sends archives of a few MiB in parts. */
  maxPutBytes = MAX_PUT_BYTES
  /** Statuses the next PUTs are answered with, in order, their bytes read and dropped. */
  readonly refuse: number[] = []
  #server: Server | null = null
  #url = ''

  async start(): Promise<void> {
    this.#server = createServer((request, response) => {
      const path = (request.url ?? '').slice(1).split('?')[0] ?? ''
      const part = /^__parts\/([^/]+)\/(\d+)$/.exec(path)
      if (request.method === 'PUT') {
        const refused = this.refuse.shift()
        if (refused !== undefined) {
          request.resume()
          request.on('end', () => {
            response.statusCode = refused
            response.end()
          })
          return
        }
        const chunks: Buffer[] = []
        request.on('data', (chunk: Buffer) => chunks.push(chunk))
        request.on('end', () => {
          const bytes = Buffer.concat(chunks)
          try {
            if (part === null) this.objects.set(decodeURIComponent(path), bytes)
            else this.putPart(part[1] ?? '', Number(part[2]), bytes)
          } catch {
            response.statusCode = 404
            return response.end()
          }
          response.setHeader('ETag', etagOf(bytes))
          response.end()
        })
        return
      }
      const bytes = this.objects.get(decodeURIComponent(path))
      if (bytes === undefined) {
        response.statusCode = 404
        return response.end()
      }
      if (request.method === 'GET') this.downloads++
      const range = /^bytes=(\d+)-(\d+)$/.exec(request.headers.range ?? '')
      const served = range === null ? bytes : bytes.subarray(Number(range[1]), Number(range[2]) + 1)
      if (range !== null) response.statusCode = 206
      response.setHeader('Content-Length', served.length)
      response.end(request.method === 'HEAD' ? undefined : served)
    })
    await new Promise<void>((resolve) => this.#server?.listen(0, '127.0.0.1', resolve))
    this.#url = `http://127.0.0.1:${(this.#server.address() as AddressInfo).port}`
  }

  close(): void {
    this.#server?.close()
  }

  newKey(kind: string, scope: { sandboxId: string; id: string }): string {
    return `${kind}/${scope.sandboxId || 'none'}/${scope.id || randomUUID()}`
  }

  async presignPut(key: string): Promise<UploadTarget> {
    return { url: `${this.#url}/${encodeURIComponent(key)}`, headers: {} }
  }

  async presignGet(key: string): Promise<DownloadTarget> {
    return { url: `${this.#url}/${encodeURIComponent(key)}` }
  }

  async archiveTarget(key: string): Promise<ArchiveTarget> {
    return {
      put: await this.presignPut(key),
      maxPutBytes: this.maxPutBytes,
      inParts: (sizeBytes, most) => this.presignParts(key, sizeBytes, most),
    }
  }

  async presignParts(key: string, sizeBytes: number, most?: number): Promise<PartsUpload> {
    const { partSize, count } = partPlan(sizeBytes, most)
    const id = randomUUID()
    this.uploads.set(id, { key, parts: new Map() })
    return {
      partSize,
      urls: Array.from({ length: count }, (_, index) => `${this.#url}/__parts/${id}/${index + 1}`),
      headers: {},
      complete: async (parts: readonly PutPart[]) => this.#complete(id, parts),
      abort: async () => {
        if (this.uploads.delete(id)) this.aborted++
      },
    }
  }

  /**
   * A part PUT to its link, as the listener takes it; for a test standing in for a node, which
   * puts parts without HTTP. Returns the part's ETag.
   */
  putPart(uploadId: string, number: number, bytes: Buffer): string {
    const upload = this.uploads.get(uploadId)
    if (upload === undefined) throw new Error(`No upload ${uploadId}`)
    upload.parts.set(number, bytes)
    return etagOf(bytes)
  }

  async ingestFromUrl(url: string, sha512: string): Promise<string> {
    const response = await fetch(url)
    if (!response.ok) throw new Error(`Fetching ${url} answered ${response.status}`)
    return this.#keep(Buffer.from(await response.arrayBuffer()), sha512)
  }

  async ingestFile(path: string, sha512: string): Promise<string> {
    return this.#keep(await readFile(path), sha512)
  }

  async head(key: string): Promise<{ sizeBytes: number } | null> {
    const bytes = this.objects.get(key)
    return bytes === undefined ? null : { sizeBytes: bytes.length }
  }

  async delete(key: string): Promise<void> {
    this.objects.delete(key)
  }

  /** As S3 joins them: the parts named, in order, each with the ETag it was put with. */
  #complete(id: string, parts: readonly PutPart[]): void {
    const upload = this.uploads.get(id)
    if (upload === undefined) throw new Error(`No upload ${id}`)
    const ordered = [...parts].sort((a, b) => a.number - b.number)
    const bytes = ordered.map((part) => {
      const held = upload.parts.get(part.number)
      if (held === undefined || etagOf(held) !== part.etag)
        throw new Error(`Part ${part.number} isn't the one put with ${part.etag}`)
      return held
    })
    this.objects.set(upload.key, Buffer.concat(bytes))
    this.uploads.delete(id)
    this.completed++
  }

  #keep(bytes: Buffer, sha512: string): string {
    const actual = createHash('sha512').update(bytes).digest('hex')
    if (actual !== sha512.toLowerCase())
      throw new ArtifactMismatch(`stored bytes are ${actual}, not ${sha512}`)
    const key = `artifacts/${actual}`
    this.objects.set(key, bytes)
    return key
  }
}
