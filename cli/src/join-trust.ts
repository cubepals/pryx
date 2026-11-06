// SPDX-License-Identifier: FSL-1.1-ALv2
/** Independent release-key authentication for fleet bootstrap; fleet data never supplies trusted keys. */
import { createPublicKey, verify } from 'node:crypto'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { type BootstrapOffer, bootstrapStatement } from '../../control/src/join-bootstrap.ts'
import { JOIN_TOKEN_PREFIX } from '../../control/src/join-token.ts'
import releaseKeys from '../../deploy/release-keys.pem' with { type: 'text' }

export function joinTarget(token: string) {
  if (
    !token.startsWith(JOIN_TOKEN_PREFIX) ||
    token.length > 64 * 1024 ||
    !/^[A-Za-z0-9_-]+$/.test(token.slice(JOIN_TOKEN_PREFIX.length))
  )
    throw new Error('Invalid join token')
  let payload: { u?: unknown; h?: unknown }
  try {
    payload = JSON.parse(Buffer.from(token.slice(JOIN_TOKEN_PREFIX.length), 'base64url').toString())
  } catch {
    throw new Error('Invalid join token')
  }
  if (typeof payload.u !== 'string' || typeof payload.h !== 'string' || !/^[a-f0-9]{64}$/.test(payload.h))
    throw new Error('Join token has no endpoint or CA pin')
  const url = new URL(payload.u)
  if (
    url.protocol !== 'https:' ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.href.replace(/\/+$/, '') !== payload.u.replace(/\/+$/, '')
  )
    throw new Error('Join endpoint must be canonical HTTPS')
  return { url: url.href.replace(/\/+$/, ''), hash: payload.h }
}
const versionParts = (value: string) => {
  if (value.length > 64 || !/^\d+\.\d+\.\d+$/.test(value)) throw new Error('Invalid bootstrap version')
  return value.split('.').map(BigInt)
}
export function verifyBootstrapOffer(value: unknown, trustedKeys = releaseKeys): BootstrapOffer {
  const offer = value as Partial<BootstrapOffer> | null
  if (
    !offer ||
    typeof offer.version !== 'string' ||
    typeof offer.sha256 !== 'string' ||
    typeof offer.signature !== 'string' ||
    !/^[a-f0-9]{128}$/i.test(offer.signature)
  )
    throw new Error('Bootstrap has no valid release signature')
  const offered = versionParts(offer.version),
    minimum = versionParts(PRYX_VERSION)
  for (let i = 0; i < minimum.length; i++) {
    if ((offered[i] ?? 0n) < (minimum[i] ?? 0n)) throw new Error('Bootstrap release is older than this CLI')
    if ((offered[i] ?? 0n) > (minimum[i] ?? 0n)) break
  }
  const statement = Buffer.from(bootstrapStatement(offer.version, offer.sha256))
  // Node crypto accepts one RFC 7468 SubjectPublicKeyInfo PEM block per createPublicKey call.
  const keys = trustedKeys.match(/-----BEGIN PUBLIC KEY-----[\s\S]*?-----END PUBLIC KEY-----/g) ?? []
  const valid = keys.some((pem) => {
    try {
      const key = createPublicKey(pem)
      return (
        key.asymmetricKeyType === 'ed25519' &&
        verify(null, statement, key, Buffer.from(offer.signature as string, 'hex'))
      )
    } catch {
      return false
    }
  })
  if (!valid) throw new Error('Bootstrap signature is not from a trusted release key')
  return offer as BootstrapOffer
}
