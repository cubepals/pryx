// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Who will use a presigned link: a sandbox runtime, from inside the provider's network, or a person's
 * browser. One address serves both in production; locally, containers and the host reach the store
 * by different names, and a link is signed for the name its user will call.
 */

import type { ArchiveTarget, DownloadTarget, UploadTarget } from './runtime-model.ts'

export type LinkAudience = 'runtime' | 'browser'

/**
 * Object storage, when a deployment has it. Generic primitives only: it knows nothing about
 * sandboxes, revisions or policy. Absent from a deployment means the archives capability is off.
 */
export interface ArchiveStore {
  /**
   * The key for a new object, laid out as this store keeps them (§4: an archive's key is issued
   * by the store). Artifacts are keyed by their bytes in `ingestFromUrl` instead.
   */
  newKey(
    kind: 'archive' | 'world_upload' | 'world_download' | 'mod_upload' | 'pack_upload',
    scope: { sandboxId: string; id: string },
  ): string
  /**
   * A link that stores one object. With `sizeBytes`, the length is part of the signature and the
   * store refuses a body of any other size, so a browser can't put more than it said it would.
   */
  presignPut(
    key: string,
    ttlSeconds: number,
    audience: LinkAudience,
    sizeBytes?: number,
  ): Promise<UploadTarget>
  presignGet(
    key: string,
    ttlSeconds: number,
    audience: LinkAudience,
    downloadName?: string,
  ): Promise<DownloadTarget>
  /**
   * Where a runtime writes an archive under `key`: a presigned PUT for one that fits, and, for one
   * larger than a PUT carries, an upload in parts begun when the runtime asks for it. Nothing is
   * stored until the upload completes; one never completed is dropped with `abort`.
   */
  archiveTarget(key: string, ttlSeconds: number, audience: LinkAudience): Promise<ArchiveTarget>
  /**
   * Copies a public URL into the store under a content-addressed key and returns the key. The
   * bytes are checked before they are stored; different bytes are an ArtifactMismatch.
   */
  ingestFromUrl(url: string, sha512: string): Promise<string>
  /**
   * Stores a file on this machine under a content-addressed key and returns the key, as
   * `ingestFromUrl` does for a URL: a pack Blockly built. Different bytes are an ArtifactMismatch.
   */
  ingestFile(path: string, sha512: string): Promise<string>
  head(key: string): Promise<{ sizeBytes: number } | null>
  delete(key: string): Promise<void>
}

/** A URL served other bytes than the ones asked for, or more of them than the store takes. */
export class ArtifactMismatch extends Error {
  constructor(detail: string) {
    super(detail)
    this.name = 'ArtifactMismatch'
  }
}
