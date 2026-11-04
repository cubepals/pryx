// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The archive store as the fleet uses it: the deployment's own store, through the same port as
 * every archive, with the fleet's copies under a prefix of their own. The store's credentials stay
 * in the control plane; a node only ever holds short-lived presigned URLs for one object.
 */

import { createHash, type Hash } from 'node:crypto'
import http from 'node:http'
import https from 'node:https'
import type { ArchiveStore } from './archive-store.ts'
import { HOUR_S, MINUTE_MS, MINUTE_S } from './durations.ts'
import type { ArchiveTarget, DownloadTarget, PutPart, UploadTarget } from './runtime-model.ts'

/** A presigned URL is checked when a transfer starts, so this only has to cover the wait to start. */
const URL_TTL_SECONDS = 30 * MINUTE_S
/** A copy in parts reads its source once per part, the last of them hours after the first. */
const RANGES_TTL_SECONDS = 12 * HOUR_S
/** A part sent this many times without an answer fails the copy, as it does on a node. */
const PART_TRIES = 3
/** A PUT that moves no bytes for this long is given up. */
const STALL_MS = 5 * MINUTE_MS

export class FleetStore {
  readonly #store: ArchiveStore
  readonly #deployment: string

  constructor(store: ArchiveStore, deployment: string) {
    this.#store = store
    this.#deployment = deployment
  }

  /** Where a copy of a server's data lives: one object per `fleet_archives` row. */
  objectKey(workload: string, archive: string): string {
    return `fleet/${this.#deployment}/${workload}/${archive}.tar.gz`
  }

  /** For a node to write a copy to: in one PUT, or in parts when it is larger than one carries. */
  target(key: string): Promise<ArchiveTarget> {
    return this.#store.archiveTarget(key, URL_TTL_SECONDS, 'runtime')
  }

  /** For a node to download from. */
  getUrl(key: string): Promise<DownloadTarget> {
    return this.#store.presignGet(key, URL_TTL_SECONDS, 'runtime')
  }

  head(key: string): Promise<{ sizeBytes: number } | null> {
    return this.#store.head(key)
  }

  delete(key: string): Promise<void> {
    return this.#store.delete(key)
  }

  /**
   * Streams an object to a target someone else presigned (the application's archive), hashing it
   * on the way, through this process: the store holds both, but a presigned link can't be the
   * target of a copy inside it. An object larger than one PUT carries goes in parts, each read
   * from the store by its range, in order, so the hash is of the whole.
   */
  async copyTo(key: string, target: ArchiveTarget): Promise<{ sizeBytes: number; sha256: string }> {
    const held = await this.#store.head(key)
    if (held === null) throw new Error(`The store holds nothing at ${key}`)
    const size = held.sizeBytes
    const source = await this.#store.presignGet(key, RANGES_TTL_SECONDS, 'browser')
    let hash = createHash('sha256')
    if (size <= target.maxPutBytes) {
      await send(source.url, null, target.put, hash)
      return { sizeBytes: size, sha256: hash.digest('hex') }
    }
    const parts = await target.inParts(size)
    try {
      const count = Math.ceil(size / parts.partSize)
      if (count > parts.urls.length)
        throw new Error(`${size} bytes need ${count} parts; the store gave ${parts.urls.length}`)
      const put: PutPart[] = []
      for (let number = 1; number <= count; number++) {
        const start = (number - 1) * parts.partSize
        const range: [number, number] = [start, Math.min(size, start + parts.partSize) - 1]
        const to = { url: parts.urls[number - 1] ?? '', headers: parts.headers }
        for (let attempt = 1; ; attempt++) {
          // The hash as it stood before this part, so a part sent again is hashed once.
          const trying = hash.copy()
          try {
            put.push({
              number,
              etag: await send(source.url, range, to, trying),
            })
            hash = trying
            break
          } catch (error) {
            if (attempt >= PART_TRIES || !passing(error)) throw error
          }
        }
      }
      await parts.complete(put)
    } catch (error) {
      await parts.abort().catch(() => undefined)
      throw error
    }
    return { sizeBytes: size, sha256: hash.digest('hex') }
  }
}

/** A request the store answered with an error, or that never got an answer (`status` 0). */
class TransferFailed extends Error {
  readonly status: number

  constructor(message: string, status: number) {
    super(message)
    this.status = status
  }
}

/** Worth sending again: the network, or the store busy; not a link it refuses. */
function passing(error: unknown): boolean {
  if (!(error instanceof TransferFailed)) return true
  return error.status === 0 || error.status === 408 || error.status === 429 || error.status >= 500
}

/**
 * Bytes of an object (all of it, or the inclusive `range`) streamed from its URL to one PUT,
 * hashed on the way. Returns the ETag the PUT was answered with. The bytes are written as they
 * come, waiting while the PUT drains; a pipe from the fetch's stream stalls under Bun.
 */
async function send(
  from: string,
  range: [number, number] | null,
  to: UploadTarget,
  hash: Hash,
): Promise<string> {
  const response = await fetch(
    from,
    range === null ? {} : { headers: { range: `bytes=${range[0]}-${range[1]}` } },
  )
  if (!response.ok || response.body === null)
    throw new TransferFailed(`The store answered ${response.status}`, response.status)
  const length = Number(response.headers.get('content-length') ?? Number.NaN)
  if (!Number.isFinite(length)) throw new Error('The store gave no length')
  if (range !== null && (response.status !== 206 || length !== range[1] - range[0] + 1))
    throw new Error(`The store gave ${length} bytes for a ${range[1] - range[0] + 1}-byte range`)
  const body = response.body
  const url = new URL(to.url)
  const request = url.protocol === 'https:' ? https.request : http.request
  return new Promise<string>((resolve, reject) => {
    const put = request(
      url,
      {
        method: 'PUT',
        headers: { ...to.headers, 'content-length': String(length) },
      },
      (answer) => {
        const status = answer.statusCode ?? 0
        answer.resume()
        if (status >= 300) {
          // Refused, perhaps before the bytes are all sent: what is left of them stays unsent.
          const refused = new TransferFailed(`The upload got HTTP ${status}`, status)
          reject(refused)
          put.destroy(refused)
          return
        }
        answer.on('error', (error) => reject(new TransferFailed(error.message, 0)))
        answer.on('end', () => resolve(String(answer.headers.etag ?? '')))
      },
    )
    put.on('error', (error) => reject(new TransferFailed(error.message, 0)))
    put.setTimeout(STALL_MS, () => put.destroy(new Error(`The upload stalled for ${STALL_MS / 1000} s`)))
    // Until the PUT takes more, or has ended: a write to one that ended never drains.
    const drained = () =>
      new Promise<void>((ready) => {
        const done = () => {
          put.off('drain', done)
          put.off('close', done)
          ready()
        }
        put.once('drain', done)
        put.once('close', done)
      })
    const write = async () => {
      let sent = 0
      // Leaving the loop early cancels the read, so the store's connection goes too.
      for await (const chunk of body as unknown as AsyncIterable<Uint8Array>) {
        if (put.destroyed) throw new TransferFailed('The upload ended before its bytes were sent', 0)
        hash.update(chunk)
        sent += chunk.length
        if (!put.write(chunk)) await drained()
      }
      if (sent !== length) throw new Error(`The store sent ${sent} of ${length} bytes`)
      put.end()
    }
    write().catch((error: unknown) => {
      put.destroy()
      reject(error instanceof TransferFailed ? error : new TransferFailed(String(error), 0))
    })
  })
}
