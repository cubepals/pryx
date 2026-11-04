// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { createHash } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import type { ReadableStream as WebReadableStream } from 'node:stream/web'
import {
  AbortMultipartUploadCommand,
  CompleteMultipartUploadCommand,
  CreateMultipartUploadCommand,
  DeleteObjectCommand,
  GetObjectCommand,
  HeadObjectCommand,
  NoSuchUpload,
  NotFound,
  PutObjectCommand,
  S3Client,
  UploadPartCommand,
} from '@aws-sdk/client-s3'
import { getSignedUrl } from '@aws-sdk/s3-request-presigner'
import { MINUTE_MS } from '@blockly/contracts/durations'
import { create as contentDisposition } from 'content-disposition'
import { type ArchiveStore, ArtifactMismatch, type LinkAudience } from '../../app/ports/optional.ts'
import type {
  ArchiveTarget,
  DownloadTarget,
  PartsUpload,
  PutPart,
  UploadTarget,
} from '../../app/ports/runtime.ts'
import { MAX_PUT_BYTES, PARTS_LINK_SECONDS, partPlan } from './parts.ts'

/**
 * Any S3-compatible store (Cloudflare R2 in production, RustFS locally) through the official AWS
 * SDK. Checksums are computed only where S3 requires them: with the SDK's default, a presigned PUT
 * carries a checksum of an empty body that its real upload can't match, which SeaweedFS refuses
 * (docs/dependency-audit.md). Path-style addressing works on every one of them.
 */

export interface S3ArchiveStoreOptions {
  /** How the control plane reaches the store; also the name browsers use. */
  endpoint: string
  /** How game runtimes reach it, when that name differs (a local Docker network). */
  runtimeEndpoint?: string | null
  bucket: string
  region: string
  accessKeyId: string
  secretAccessKey: string
  /** The most an ingest copies before giving up. */
  maxIngestBytes?: number
  fetch?: (url: string) => Promise<Response>
  /**
   * How long to wait for a connection, and for any byte while a request is under way. The SDK has
   * neither by default, so a store that stops answering would hold a request forever.
   */
  timeouts?: { connectMs: number; idleMs: number }
}

const SHA512 = /^[0-9a-f]{128}$/

export class S3ArchiveStore implements ArchiveStore {
  readonly #s3: S3Client
  readonly #signers: Record<LinkAudience, S3Client>
  readonly #bucket: string
  readonly #maxIngestBytes: number
  readonly #fetch: (url: string) => Promise<Response>

  constructor(options: S3ArchiveStoreOptions) {
    const client = (endpoint: string) =>
      new S3Client({
        endpoint,
        region: options.region,
        forcePathStyle: true,
        credentials: { accessKeyId: options.accessKeyId, secretAccessKey: options.secretAccessKey },
        requestChecksumCalculation: 'WHEN_REQUIRED',
        responseChecksumValidation: 'WHEN_REQUIRED',
        // An upload may take long, so no limit on the whole request: a minute without a byte fails it.
        requestHandler: {
          connectionTimeout: options.timeouts?.connectMs ?? 10_000,
          socketTimeout: options.timeouts?.idleMs ?? MINUTE_MS,
        },
      })
    this.#s3 = client(options.endpoint)
    this.#signers = {
      browser: this.#s3,
      runtime: options.runtimeEndpoint ? client(options.runtimeEndpoint) : this.#s3,
    }
    this.#bucket = options.bucket
    this.#maxIngestBytes = options.maxIngestBytes ?? 512 * 1024 * 1024
    this.#fetch = options.fetch ?? ((url) => fetch(url))
  }

  async presignPut(
    key: string,
    ttlSeconds: number,
    audience: LinkAudience,
    sizeBytes?: number,
  ): Promise<UploadTarget> {
    const command = new PutObjectCommand({
      Bucket: this.#bucket,
      Key: key,
      ...(sizeBytes === undefined ? {} : { ContentLength: sizeBytes }),
    })
    return {
      url: await getSignedUrl(this.#signers[audience], command, {
        expiresIn: ttlSeconds,
        // The length is signed; browsers send it themselves, so it needs no header of ours.
        ...(sizeBytes === undefined ? {} : { signableHeaders: new Set(['content-length']) }),
      }),
      headers: {},
    }
  }

  async presignGet(
    key: string,
    ttlSeconds: number,
    audience: LinkAudience,
    downloadName?: string,
  ): Promise<DownloadTarget> {
    const command = new GetObjectCommand({
      Bucket: this.#bucket,
      Key: key,
      ...(downloadName ? { ResponseContentDisposition: contentDisposition(downloadName) } : {}),
    })
    return { url: await getSignedUrl(this.#signers[audience], command, { expiresIn: ttlSeconds }) }
  }

  /**
   * The bytes go to a temporary file while they are hashed, and reach the store only once they
   * match. Content addressing makes a second ingest of the same bytes a no-op.
   */
  async ingestFromUrl(url: string, sha512: string): Promise<string> {
    const expected = sha512.toLowerCase()
    if (!SHA512.test(expected)) throw new Error(`Not a sha512: ${sha512}`)
    const key = `artifacts/${expected}`
    if ((await this.head(key)) !== null) return key

    const response = await this.#fetch(url)
    if (!response.ok || response.body === null) throw new Error(`Fetching ${url} answered ${response.status}`)
    const declared = Number(response.headers.get('content-length'))
    if (declared > this.#maxIngestBytes)
      throw new ArtifactMismatch(
        `${url} is ${declared} bytes, more than the ${this.#maxIngestBytes} an ingest takes`,
      )

    const dir = await mkdtemp(join(tmpdir(), 'blockly-ingest-'))
    try {
      const file = join(dir, 'blob')
      const hash = createHash('sha512')
      let size = 0
      const limit = this.#maxIngestBytes
      await pipeline(
        Readable.fromWeb(response.body as WebReadableStream<Uint8Array>),
        new Transform({
          transform(chunk: Buffer, _encoding, done) {
            size += chunk.length
            if (size > limit)
              return done(new ArtifactMismatch(`${url} is more than the ${limit} bytes an ingest takes`))
            hash.update(chunk)
            done(null, chunk)
          },
        }),
        createWriteStream(file),
      )
      const actual = hash.digest('hex')
      if (actual !== expected) throw new ArtifactMismatch(`${url} served sha512 ${actual}, not ${expected}`)
      await this.#s3.send(
        new PutObjectCommand({
          Bucket: this.#bucket,
          Key: key,
          Body: createReadStream(file),
          ContentLength: size,
        }),
      )
      return key
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  }

  /** A local file is hashed again before it is stored, so the key always names its bytes. */
  async ingestFile(path: string, sha512: string): Promise<string> {
    const expected = sha512.toLowerCase()
    if (!SHA512.test(expected)) throw new Error(`Not a sha512: ${sha512}`)
    const key = `artifacts/${expected}`
    if ((await this.head(key)) !== null) return key
    const hash = createHash('sha512')
    let size = 0
    await pipeline(
      createReadStream(path),
      new Transform({
        transform(chunk: Buffer, _encoding, done) {
          size += chunk.length
          hash.update(chunk)
          done()
        },
      }),
    )
    const actual = hash.digest('hex')
    if (actual !== expected) throw new ArtifactMismatch(`${path} holds sha512 ${actual}, not ${expected}`)
    await this.#s3.send(
      new PutObjectCommand({
        Bucket: this.#bucket,
        Key: key,
        Body: createReadStream(path),
        ContentLength: size,
      }),
    )
    return key
  }

  newKey(
    kind: 'archive' | 'world_upload' | 'world_download' | 'mod_upload' | 'pack_upload',
    { serverId, id }: { serverId: string; id: string },
  ): string {
    switch (kind) {
      case 'archive':
        return `archives/${serverId}/${id}.tar.gz`
      case 'world_upload':
        // A world brought before its server exists has no server to sit under. An empty segment
        // (`archives//…`) is a key some stores refuse, RustFS among them.
        return serverId === '' ? `uploads/worlds/${id}.tar.gz` : `archives/${serverId}/uploads/${id}.tar.gz`
      case 'world_download':
        return `archives/${serverId}/downloads/${id}.tar.gz`
      case 'mod_upload':
      case 'pack_upload':
        return `uploads/staging/${id}`
    }
  }

  async head(key: string): Promise<{ sizeBytes: number } | null> {
    try {
      const found = await this.#s3.send(new HeadObjectCommand({ Bucket: this.#bucket, Key: key }))
      return { sizeBytes: found.ContentLength ?? 0 }
    } catch (error) {
      if (error instanceof NotFound) return null
      throw error
    }
  }

  /** Deleting what isn't there succeeds, as S3 defines it. */
  async delete(key: string): Promise<void> {
    await this.#s3.send(new DeleteObjectCommand({ Bucket: this.#bucket, Key: key }))
  }

  async archiveTarget(key: string, ttlSeconds: number, audience: LinkAudience): Promise<ArchiveTarget> {
    return {
      put: await this.presignPut(key, ttlSeconds, audience),
      maxPutBytes: MAX_PUT_BYTES,
      inParts: (sizeBytes, most) => this.#presignParts(key, sizeBytes, audience, most),
    }
  }

  /**
   * S3's multipart upload: begun here, each part PUT by whoever holds its link, and joined here
   * from the parts' ETags. R2 needs every part but the last to be the same size, which the plan
   * gives. An upload never completed stores nothing, and `abort` frees what its parts took.
   */
  async #presignParts(
    key: string,
    sizeBytes: number,
    audience: LinkAudience,
    most: number | undefined,
  ): Promise<PartsUpload> {
    const { partSize, count } = partPlan(sizeBytes, most)
    const begun = await this.#s3.send(new CreateMultipartUploadCommand({ Bucket: this.#bucket, Key: key }))
    const uploadId = begun.UploadId
    if (!uploadId) throw new Error(`The store began no upload for ${key}`)
    const urls = await Promise.all(
      Array.from({ length: count }, (_, index) =>
        getSignedUrl(
          this.#signers[audience],
          new UploadPartCommand({
            Bucket: this.#bucket,
            Key: key,
            UploadId: uploadId,
            PartNumber: index + 1,
          }),
          { expiresIn: PARTS_LINK_SECONDS },
        ),
      ),
    )
    let ended = false
    return {
      partSize,
      urls,
      headers: {},
      complete: async (parts: readonly PutPart[]) => {
        await this.#s3.send(
          new CompleteMultipartUploadCommand({
            Bucket: this.#bucket,
            Key: key,
            UploadId: uploadId,
            MultipartUpload: {
              Parts: [...parts]
                .sort((a, b) => a.number - b.number)
                .map((part) => ({ PartNumber: part.number, ETag: part.etag })),
            },
          }),
        )
        ended = true
      },
      abort: async () => {
        if (ended) return
        ended = true
        try {
          await this.#s3.send(
            new AbortMultipartUploadCommand({ Bucket: this.#bucket, Key: key, UploadId: uploadId }),
          )
        } catch (error) {
          if (!(error instanceof NoSuchUpload)) throw error
        }
      },
    }
  }
}
