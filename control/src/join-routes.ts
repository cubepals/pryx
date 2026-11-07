// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What a new host fetches from the node endpoint before it has an identity, with no
 * authentication: nothing here is secret, and the host checks each piece before using it
 * (docs/fleet-operations.md, "Adding a node"; pryxd's docs/protocol.md).
 *
 * - `GET /fleet/v1/ca.pem`: the fleet CA, fetched without trusting the connection and checked
 *   against the join token's hash.
 * - `GET /fleet/v1/join.sh`: join.sh, beside this file, with daemon.json written into it.
 * - `GET /fleet/v1/bootstrap.sh` and `…/bootstrap.json`: release-authenticated bootstrap and
 *   detached signature metadata. The independently installed CLI verifies before execution.
 * - `GET /fleet/v1/blocklyd` and `…/blocklyd.sha256`: the static pryxd this control plane's
 *   image carries, and its hash in `sha256sum -c` form. A process without it starts as usual
 *   and answers 404, saying so.
 * - `GET /fleet/v1/blocklyd.json`: that pryxd as an upgrade offer, version, sha256 and release
 *   signature, which an enrolled node's `pryxd upgrade` asks for.
 *
 * `pryxdRelease` says which version that pryxd is, and its release's signature, for the
 * upgrade rollout (registry/upgrades.ts). The signature is the release's `pryxd.sig`, which the
 * image carries beside the binary; the control plane can't make one, and only passes it on.
 * Enrollment and what follows it are node-endpoint.ts.
 */

import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { promisify } from 'node:util'
import { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import nodeSchema from '../../openapi/node-writes.openapi.json' with { type: 'json' }
import { SECOND_MS } from './durations.ts'
import { BOOTSTRAP_SIGNATURE_SUFFIX, bootstrapOffer, joinBootstrapScript } from './join-bootstrap.ts'
import type { UpgradeOffer } from './wire.ts'

/** An Ed25519 signature's length: `pryxd.sig` is one, raw, as `openssl pkeyutl -sign` writes it. */
const SIGNATURE_BYTES = 64

/**
 * The release's signature beside `bin` (`<bin>.sig`), in hex; null when there is none, saying so
 * when there is a file that isn't one.
 */
async function signatureOf(bin: string, suffix = '.sig'): Promise<string | null> {
  const signature = await readFile(`${bin}${suffix}`).catch(() => null)
  if (signature === null) return null
  if (signature.length === SIGNATURE_BYTES) return signature.toString('hex')
  console.error(`fleet: ${bin}.sig is ${signature.length} bytes, not a signature, so it isn't passed on`)
  return null
}

/**
 * The version (from its `--version`), sha256 and release signature of the pryxd at `bin`, read
 * once as the process starts; null, saying why, when there is none or it doesn't run here.
 */
export async function pryxdRelease(bin: string): Promise<UpgradeOffer | null> {
  const binary = await readFile(bin).catch(() => null)
  if (binary === null) return null
  const printed = await promisify(execFile)(bin, ['--version'], {
    timeout: 10 * SECOND_MS,
  }).catch((error: Error) => {
    console.error(`fleet: ${bin} --version failed, so no node is offered it: ${error.message}`)
    return null
  })
  const version = /^pryxd (\S+)\s*$/.exec(printed?.stdout ?? '')?.[1]
  if (printed !== null && version === undefined)
    console.error(`fleet: ${bin} --version printed no version, so no node is offered it`)
  if (version === undefined) return null
  const signature = await signatureOf(bin)
  return {
    version,
    sha256: createHash('sha256').update(binary).digest('hex'),
    ...(signature === null ? {} : { signature }),
  }
}

export function joinRoutes(options: {
  caPem: string
  pryxdBin: string
  agentBin?: string
  /** A detached signature file carries no signing authority; the host independently verifies it. */
  bootstrapSignatureFile?: string
  /** `pryxdRelease(pryxdBin)`, read as the process started. */
  release: UpgradeOffer | null
}): Hono {
  const app = new Hono()
  const agentBin = options.agentBin ?? nodeSchema['x-pryx-agent'].defaultBinary
  const agentInstall = existsSync(agentBin)
    ? `
curl -fsS --cacert "$ca" "$url/fleet/v1/pryx-agent" -o "$dir/pryx-agent" || fail "pryx-agent could not be downloaded"
curl -fsS --cacert "$ca" "$url/fleet/v1/pryx-agent.sha256" -o "$dir/pryx-agent.sha256"
(cd "$dir" && sha256sum -c --quiet pryx-agent.sha256) || fail "pryx-agent checksum verification failed"
install -D -m 0755 "$dir/pryx-agent" '${nodeSchema['x-pryx-agent'].defaultBinary}'
`
    : ''
  // Read once, from beside this file: join.sh is the control plane's, and daemon.json is the pinned
  // pryxd release's (scripts/blocklyd.ts).
  const daemonJson = readFileSync(
    process.env.PRYX_ASSET_DIR
      ? join(process.env.PRYX_ASSET_DIR, 'daemon.json')
      : new URL('./daemon.json', import.meta.url),
    'utf8',
  )
  const daemonFetch = String.raw`dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
curl -sS --fail-with-body --cacert "$ca" "$url/fleet/v1/blocklyd" -o "$dir/pryxd" ||
  fail "pryxd couldn't be downloaded: $(head -c 300 "$dir/pryxd" 2>/dev/null)"
curl -fsS --cacert "$ca" "$url/fleet/v1/blocklyd.sha256" -o "$dir/pryxd.sha256"
(cd "$dir" && sha256sum -c --quiet pryxd.sha256) || fail "the pryxd downloaded doesn't match its sha256, so it wasn't installed"`
  const template = readFileSync(
    process.env.PRYX_ASSET_DIR
      ? join(process.env.PRYX_ASSET_DIR, 'join.sh')
      : new URL('./join.sh', import.meta.url),
    'utf8',
  )
  const script = template
    .replace('@DAEMON_JSON@', daemonJson.trimEnd())
    .replace('@AGENT_INSTALL@', agentInstall)
    .replace('@DAEMON_FETCH@', daemonFetch)
    .replace('@ROOT_CHECK@', '[ "$(id -u)" = 0 ] || fail "run it as root"')
    .replace('@PREPARE@', '')
  const verifiedScript =
    options.release && existsSync(agentBin)
      ? joinBootstrapScript(
          template,
          daemonJson,
          options.release.sha256,
          createHash('sha256').update(readFileSync(agentBin)).digest('hex'),
        )
      : null
  const missing = (c: { text: (text: string, status: typeof StatusCodes.NOT_FOUND) => Response }) =>
    c.text(
      `This control plane has no pryxd to hand out: ${options.pryxdBin} is missing. Its image carries ` +
        'one (apps/control/Dockerfile); elsewhere, set PRYX_PRYXD_BIN to a static pryxd.\n',
      StatusCodes.NOT_FOUND,
    )

  app.get('/fleet/v1/ca.pem', (c) =>
    c.body(options.caPem, StatusCodes.OK, { 'content-type': 'application/x-pem-file' }),
  )
  app.get('/fleet/v1/join.sh', (c) =>
    c.body(script, StatusCodes.OK, { 'content-type': 'text/x-shellscript' }),
  )
  app.get('/fleet/v1/bootstrap.sh', (c) =>
    verifiedScript === null
      ? missing(c)
      : c.body(verifiedScript, StatusCodes.OK, { 'content-type': 'text/x-shellscript' }),
  )
  app.get('/fleet/v1/bootstrap.json', async (c) =>
    verifiedScript === null
      ? missing(c)
      : c.json(
          bootstrapOffer(
            verifiedScript,
            (await signatureOf(
              options.bootstrapSignatureFile ?? `${options.pryxdBin}${BOOTSTRAP_SIGNATURE_SUFFIX}`,
              '',
            )) ?? undefined,
          ),
        ),
  )
  app.get('/fleet/v1/blocklyd', async (c) => {
    const binary = await readFile(options.pryxdBin).catch(() => null)
    if (binary === null) return missing(c)
    return c.body(new Uint8Array(binary), 200, {
      'content-type': 'application/octet-stream',
    })
  })
  app.get('/fleet/v1/blocklyd.sha256', async (c) => {
    const binary = await readFile(options.pryxdBin).catch(() => null)
    if (binary === null) return missing(c)
    return c.text(`${createHash('sha256').update(binary).digest('hex')}  pryxd\n`)
  })
  app.get('/fleet/v1/blocklyd.json', (c) => (options.release === null ? missing(c) : c.json(options.release)))
  for (const checksum of [false, true])
    app.get(`/fleet/v1/pryx-agent${checksum ? '.sha256' : ''}`, async (c) => {
      const binary = await readFile(agentBin).catch(() => null)
      if (binary === null)
        return c.text('This control plane has no static agent; set PRYX_AGENT_BIN.\n', StatusCodes.NOT_FOUND)
      return checksum
        ? c.text(`${createHash('sha256').update(binary).digest('hex')}  pryx-agent\n`)
        : c.body(new Uint8Array(binary), StatusCodes.OK, { 'content-type': 'application/octet-stream' })
    })
  return app
}
