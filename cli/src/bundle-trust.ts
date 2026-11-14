// SPDX-License-Identifier: FSL-1.1-ALv2
/** Authenticate the complete archive before extraction. Trust comes from the compiled release
 * keys, never an environment variable, download server, archive or fleet endpoint. */
import { createPublicKey, verify } from 'node:crypto'
import { bundleStatement } from '../../control/src/bundle-release.ts'
import productionKeys from '../../deploy/release-keys.pem' with { type: 'text' }

// An isolated CI compilation can substitute a fixture public key. Normal release builds do
// not define this symbol; no runtime option or environment can enable fixture trust.
declare const PRYX_PREVIEW_PUBLIC_KEY: string | undefined
const compiledKeys = typeof PRYX_PREVIEW_PUBLIC_KEY === 'undefined' ? productionKeys : PRYX_PREVIEW_PUBLIC_KEY

export function verifyBundleSignature(
  version: string,
  digest: string,
  signature: Uint8Array,
  keys = compiledKeys,
) {
  if (signature.length !== 64) throw new Error('Host bundle signature must be Ed25519')
  const statement = Buffer.from(bundleStatement(version, digest))
  const valid = (keys.match(/-----BEGIN PUBLIC KEY-----[\s\S]*?-----END PUBLIC KEY-----/g) ?? []).some(
    (pem) => {
      const key = createPublicKey(pem)
      return key.asymmetricKeyType === 'ed25519' && verify(null, statement, key, signature)
    },
  )
  if (!valid) throw new Error('Host bundle signature is not from a trusted release key; nothing was unpacked')
}
