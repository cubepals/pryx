// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Conditional store effects and presigning for the optional lease. HTTP Date, never the control host clock, supplies authority. */
import { GetObjectCommand, PutObjectCommand, S3Client } from '@aws-sdk/client-s3'
import { getSignedUrl } from '@aws-sdk/s3-request-presigner'
import { StatusCodes } from 'http-status-codes'
import { DAY_S, SECOND_MS } from './durations.ts'
import type { components } from './generated/node-reads.ts'
import type { S3ArchiveStoreOptions } from './store.ts'
export type LeaseObject = components['schemas']['StoreLeaseObject']
export interface LeaseRead {
  object: LeaseObject | null
  etag: string | null
  now: number
}
export interface LeaseBucket {
  read(key: string): Promise<LeaseRead>
  compare(key: string, etag: string | null, object: LeaseObject): Promise<boolean>
  links(key: string): Promise<{ readUrl: string; writeUrl: string }>
}
export class S3LeaseBucket implements LeaseBucket {
  readonly local: S3Client
  readonly remote: S3Client
  constructor(readonly options: S3ArchiveStoreOptions) {
    const client = (endpoint: string) =>
      new S3Client({
        endpoint,
        region: options.region,
        forcePathStyle: true,
        credentials: { accessKeyId: options.accessKeyId, secretAccessKey: options.secretAccessKey },
        requestChecksumCalculation: 'WHEN_REQUIRED',
        responseChecksumValidation: 'WHEN_REQUIRED',
      })
    this.local = client(options.endpoint)
    this.remote = options.runtimeEndpoint ? client(options.runtimeEndpoint) : this.local
  }
  private async url(key: string, method: 'GET' | 'PUT', remote = false) {
    const args = { Bucket: this.options.bucket, Key: key }
    // If-Match stays an unsigned HTTPS header because its ETag changes after each renewal.
    return getSignedUrl(
      remote ? this.remote : this.local,
      method === 'GET' ? new GetObjectCommand(args) : new PutObjectCommand(args),
      { expiresIn: DAY_S },
    )
  }
  async read(key: string): Promise<LeaseRead> {
    const response = await fetch(await this.url(key, 'GET'), {
      signal: AbortSignal.timeout(5 * SECOND_MS),
      cache: 'no-store',
      headers: { 'cache-control': 'no-cache, no-store' },
      redirect: 'error',
    })
    if (Number(response.headers.get('age') ?? 0) > 0) {
      await response.body?.cancel()
      throw new Error('Cached lease authority refused')
    }
    const now = Date.parse(response.headers.get('date') ?? '')
    if (!Number.isFinite(now)) throw new Error('Lease store must supply HTTP Date')
    if (response.status === StatusCodes.NOT_FOUND) {
      await response.body?.cancel()
      return { object: null, etag: null, now }
    }
    if (!response.ok) throw new Error('Lease store read refused')
    const etag = response.headers.get('etag')
    if (!etag) throw new Error('Lease store must supply ETag')
    const body = await response.text()
    if (body.length > 4096) throw new Error('Lease object is too large')
    return { object: JSON.parse(body) as LeaseObject, etag, now }
  }
  async compare(key: string, etag: string | null, object: LeaseObject) {
    const response = await fetch(await this.url(key, 'PUT'), {
      method: 'PUT',
      redirect: 'error',
      headers: {
        'content-type': 'application/json',
        ...(etag ? { 'if-match': etag } : { 'if-none-match': '*' }),
      },
      body: JSON.stringify(object),
      signal: AbortSignal.timeout(5 * SECOND_MS),
    })
    await response.body?.cancel()
    if (response.status === StatusCodes.PRECONDITION_FAILED || response.status === StatusCodes.CONFLICT)
      return false
    if (!response.ok) throw new Error('Conditional lease store write refused')
    return true
  }
  async links(key: string) {
    return { readUrl: await this.url(key, 'GET', true), writeUrl: await this.url(key, 'PUT', true) }
  }
  async probe(prefix: string) {
    const key = `${prefix}/probe/${crypto.randomUUID()}`
    const object: LeaseObject = {
      session: 'probe',
      version: 1,
      workload: 'probe',
      node: 'probe',
      epoch: 0,
      generation: crypto.randomUUID(),
      expiresAtMs: 0,
    }
    if (!(await this.compare(key, null, object))) throw new Error('Lease conditional create failed')
    if (await this.compare(key, null, object)) throw new Error('Store ignores conditional create')
    const first = await this.read(key)
    if (!(await this.compare(key, first.etag, { ...object, generation: crypto.randomUUID() })))
      throw new Error('Lease conditional update failed')
    if (await this.compare(key, first.etag, object)) throw new Error('Store ignores stale If-Match')
    // The unique expired probe is retained; operators must never lifecycle-delete active lease objects.
  }
}
