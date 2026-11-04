// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * How an archive goes to an S3-compatible store: in one PUT up to the strictest store's limit, in
 * parts above it. Cloudflare R2 refuses a single PUT over 5 GiB less 5 MiB, and S3 over 5 GiB; a
 * multipart upload takes up to 10,000 parts of 5 MiB to 5 GiB each, every part but the last the
 * same size on R2.
 */

import { HOUR_S } from '@blockly/contracts/durations'

const MIB = 1024 ** 2

/** The most one PUT carries on the strictest store Blockly uses (R2). */
export const MAX_PUT_BYTES = 5 * 1024 ** 3 - 5 * MIB

/**
 * Links for parts last this long: the parts go one after another, so the last may start hours
 * after the first, and a link is checked when its request starts.
 */
export const PARTS_LINK_SECONDS = 12 * HOUR_S

/** Each part but the last: S3 takes none under 5 MiB, and none over 5 GiB (R2 a little less). */
const MIN_PART_BYTES = 8 * MIB
const MAX_PART_BYTES = 4 * 1024 ** 3
/** Parts are about this many: few enough that their links fit a request, small enough to resend. */
const PARTS_AIMED = 64
const MAX_PARTS = 10_000

/**
 * How an object of `sizeBytes` goes in parts: about `aimed` of them (64 unless asked for fewer),
 * of 8 MiB to 4 GiB each and a whole number of MiB, with room for a little more than asked, since
 * an archive packed again may come out a little larger.
 */
export function partPlan(sizeBytes: number, aimed = PARTS_AIMED): { partSize: number; count: number } {
  const room = sizeBytes + Math.floor(sizeBytes / 32) + MIB
  const even = Math.ceil(room / Math.max(1, aimed) / MIB) * MIB
  const partSize = Math.min(MAX_PART_BYTES, Math.max(MIN_PART_BYTES, even))
  const count = Math.max(1, Math.ceil(room / partSize))
  if (count > MAX_PARTS) throw new Error(`${sizeBytes} bytes is more than ${MAX_PARTS} parts hold`)
  return { partSize, count }
}
