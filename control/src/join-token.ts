// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The token an operator gives a new host, and the line it is pasted in (docs/fleet-operations.md,
 * "Adding a node"). The token is `bk1.` and base64url JSON: the node endpoint's URL (`u`), the
 * sha256 of the fleet CA as that endpoint serves it (`h`), the deployment (`d`), the one-time
 * secret (`s`) and, for a token that re-enrolls a node, that node's id (`n`). Only the secret is
 * stored, hashed, and looked up at enrollment, as a bare token always was; the region, labels and
 * the node to re-enroll stay on the token's row, and decide. `n` only lets `pryxd join` tell
 * the host it is pasted on apart from another node's.
 *
 * pryxd reads the same form in src/fleet/token.rs in cubepals/pryx.
 */

import { createHash } from 'node:crypto'

const JOIN_TOKEN_PREFIX = 'bk1.'

/** What a host needs to find the control plane and trust it before anything else. */
export interface JoinPoint {
  /** The node endpoint, as hosts reach it: `https://host:port`. */
  url: string
  caPem: string
  deployment: string
}

/**
 * sha256 of the CA certificate, byte for byte as `/fleet/v1/ca.pem` serves it, so `sha256sum -c`
 * can check it on a host with nothing else installed.
 */
export const caSha256 = (caPem: string) => createHash('sha256').update(caPem).digest('hex')

export function joinToken(join: JoinPoint, secret: string, node?: string): string {
  const payload = {
    u: join.url,
    h: caSha256(join.caPem),
    d: join.deployment,
    s: secret,
    ...(node ? { n: node } : {}),
  }
  return `${JOIN_TOKEN_PREFIX}${Buffer.from(JSON.stringify(payload)).toString('base64url')}`
}

/** What enrollment looks up: a join token's secret, or a bare token whole. */
export function secretOf(token: string): string {
  if (!token.startsWith(JOIN_TOKEN_PREFIX)) return token
  try {
    const payload = JSON.parse(Buffer.from(token.slice(JOIN_TOKEN_PREFIX.length), 'base64url').toString())
    return typeof payload?.s === 'string' ? payload.s : token
  } catch {
    return token
  }
}

/**
 * The line an operator pastes on a host, as root: a new one, or one a token re-enrolls. It fetches the CA without trusting it (it
 * isn't yet), stops unless its sha256 is the token's, then fetches join.sh over TLS checked
 * against that CA and runs it with the token: nothing runs before the hash matches.
 */
export function joinCommand(join: JoinPoint, token: string): string {
  const base = join.url.replace(/\/+$/, '')
  return [
    'd=$(mktemp -d)',
    `curl -fsSk '${base}/fleet/v1/ca.pem' -o "$d/ca.pem"`,
    `echo "${caSha256(join.caPem)}  $d/ca.pem" | sha256sum -c --quiet`,
    `curl -fsS --cacert "$d/ca.pem" '${base}/fleet/v1/join.sh' | sh -s -- ${token} "$d/ca.pem"`,
  ].join(' && ')
}
