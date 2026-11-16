// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The image allowlist a server install gives its node when `--allow-image` names nothing, and
 * how a later run of the server preset (its upgrade) moves that default to the new release's.
 * It never touches a list the operator chose or changed; the node's policy is in pryxd. */
import { existsSync, readFileSync } from 'node:fs'
import vmImages from '../../templates/vm-tools/image-inputs.json'
import { writeChanged } from './up-files.ts'

/** The node configuration `pryxd join` writes, and where the preset records the default it gave. */
const NODE_CONFIG = '/etc/pryxd/pryxd.toml'
const DEFAULTS_RECORD = '/etc/pryx/node-images.json'
const ALLOWED_LINE = /^allowed_images = .*$/m

/** The alpine and node prefixes, and, when the install carries the release's VM descriptor,
 * exactly the images that VM bundle is proven with, each by its full digest reference, which
 * admits that digest and no other. */
export function defaultAllowedImages(vmDescriptor: boolean) {
  return ['alpine:', 'node:', ...(vmDescriptor ? vmImages.map((image) => image.reference) : [])]
}

const same = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((value, index) => value === b[index])

/** The node configuration with `previous` replaced by `next`, or undefined when it does not
 * hold exactly `previous` as its allowlist: an operator's own list is never rewritten. */
export function replaceDefaultImages(text: string, previous: readonly string[], next: readonly string[]) {
  const allowed = (Bun.TOML.parse(text) as { workloads?: { allowed_images?: unknown } }).workloads
    ?.allowed_images
  if (!Array.isArray(allowed) || !same(allowed, previous) || same(previous, next)) return undefined
  if ((text.match(new RegExp(ALLOWED_LINE.source, 'gm')) ?? []).length !== 1) return undefined
  const replaced = text.replace(ALLOWED_LINE, `allowed_images = ${JSON.stringify(next)}`)
  const written = (Bun.TOML.parse(replaced) as { workloads?: { allowed_images?: unknown } }).workloads
  if (!Array.isArray(written?.allowed_images) || !same(written.allowed_images, next))
    throw new Error('The node allowlist could not be rewritten exactly')
  return replaced
}

/** Records the default this install gave `pryxd join`, so a later run can tell it from the
 * operator's own list. An install with `--allow-image` records nothing. */
export function recordDefaultImages(images: readonly string[], record = DEFAULTS_RECORD) {
  writeChanged(record, `${JSON.stringify({ defaults: images })}\n`, 0o600)
}

/** On a later run of the server preset: moves a node still holding exactly the recorded default
 * to this release's default. Says whether the node's configuration changed. */
export function refreshDefaultImages(
  next: readonly string[],
  config = NODE_CONFIG,
  record = DEFAULTS_RECORD,
) {
  if (!existsSync(record) || !existsSync(config)) return false
  const previous = (JSON.parse(readFileSync(record, 'utf8')) as { defaults?: unknown }).defaults
  if (!Array.isArray(previous) || !previous.every((entry) => typeof entry === 'string')) return false
  const replaced = replaceDefaultImages(readFileSync(config, 'utf8'), previous, next)
  if (replaced === undefined) return false
  writeChanged(config, replaced, 0o644)
  recordDefaultImages(next, record)
  return true
}
