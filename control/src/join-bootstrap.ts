// SPDX-License-Identifier: FSL-1.1-ALv2
/** Release-authenticated fleet bootstrap bytes and their signing statement; no runtime signing key. */
import { createHash } from 'node:crypto'
import nodeSchema from '../../openapi/node-writes.openapi.json' with { type: 'json' }
import { MINUTE_S } from './durations.ts'
import { PRYX_VERSION } from './generated/release.ts'
import type { UpgradeOffer } from './wire.ts'

export type BootstrapOffer = UpgradeOffer
export const bootstrapHash = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex')
export function bootstrapStatement(version: string, sha256: string) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !/^[a-f0-9]{64}$/.test(sha256))
    throw new Error('Invalid bootstrap release statement')
  return `pryx join bootstrap\n${version}\n${sha256}\n`
}
export const BOOTSTRAP_SIGNATURE_SUFFIX = '.bootstrap.sig'
export const BOOTSTRAP_SIGNATURE_ASSET = `pryxd${BOOTSTRAP_SIGNATURE_SUFFIX}`
// Bounded host staging, including endpoints that lie about Content-Length; never a code-size claim.
const MAX_BOOTSTRAP_BINARY_BYTES = 128 * 1024 ** 2

export function joinBootstrapScript(
  template: string,
  daemonJson: string,
  daemonHash: string,
  agentHash: string,
) {
  for (const hash of [daemonHash, agentHash])
    if (!/^[a-f0-9]{64}$/.test(hash)) throw new Error('Invalid bootstrap asset digest')
  const prepare = String.raw`dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT HUP INT TERM
curl --proto '=https' --max-time ${5 * MINUTE_S} --max-filesize ${MAX_BOOTSTRAP_BINARY_BYTES} -fsS --cacert "$ca" "$url/fleet/v1/blocklyd" | head -c ${MAX_BOOTSTRAP_BINARY_BYTES + 1} > "$dir/pryxd"
curl --proto '=https' --max-time ${5 * MINUTE_S} --max-filesize ${MAX_BOOTSTRAP_BINARY_BYTES} -fsS --cacert "$ca" "$url/fleet/v1/pryx-agent" | head -c ${MAX_BOOTSTRAP_BINARY_BYTES + 1} > "$dir/pryx-agent"
printf '%s  pryxd\n%s  pryx-agent\n' '${daemonHash}' '${agentHash}' > "$dir/assets.sha256"
(cd "$dir" && sha256sum -c --quiet assets.sha256) || fail "release asset verification failed; nothing was installed or executed"
if [ "$#" -gt 0 ] && [ "$1" = --verify-only ]; then exit 0; fi
[ "$(id -u)" = 0 ] || fail "run it as root"`
  return template
    .replace('@DAEMON_JSON@', daemonJson.trimEnd())
    .replace('@DAEMON_FETCH@', '')
    .replace('@ROOT_CHECK@', '')
    .replace('@PREPARE@', prepare)
    .replace(
      '@AGENT_INSTALL@',
      `install -D -m 0755 "$dir/pryx-agent" '${nodeSchema['x-pryx-agent'].defaultBinary}'`,
    )
}
export function bootstrapOffer(script: string, signature?: string): BootstrapOffer {
  return { version: PRYX_VERSION, sha256: bootstrapHash(script), ...(signature ? { signature } : {}) }
}
