// SPDX-License-Identifier: FSL-1.1-ALv2
/** Authenticate bootstrap code with compiled release keys before invoking it on the host.
 * Parts (`./`): `join-trust.ts` owns release/token validation; `join-download.ts` owns bounded TLS reads.
 */
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { bootstrapHash } from '../../control/src/join-bootstrap.ts'
import { pinnedJoinDownloads } from './join-download.ts'
import { joinTarget, verifyBootstrapOffer } from './join-trust.ts'

/** The most a bootstrap offer (`bootstrap.json`) may be. */
export const BOOTSTRAP_OFFER_BYTES = 16 * 1024

export async function joinHost(
  token: string,
  options: {
    /** Explicit local trust for isolated callers/tests; never obtained from token, endpoint or environment. */
    trustedKeys?: string
    verifyOnly?: boolean
    args?: string[]
  } = {},
) {
  const target = joinTarget(token)
  const downloads = await pinnedJoinDownloads(target.url, target.hash)
  const metadata = JSON.parse((await downloads.get('bootstrap.json', BOOTSTRAP_OFFER_BYTES)).toString())
  const offer = verifyBootstrapOffer(metadata, options.trustedKeys)
  const script = await downloads.get('bootstrap.sh', 256 * 1024)
  if (bootstrapHash(script) !== offer.sha256) throw new Error('Bootstrap digest verification failed')
  const dir = mkdtempSync(join(tmpdir(), 'pryx-verified-join-'))
  try {
    const file = join(dir, 'bootstrap.sh'),
      ca = join(dir, 'ca.pem')
    writeFileSync(file, script, { mode: 0o600, flag: 'wx' })
    writeFileSync(ca, downloads.ca, { mode: 0o600, flag: 'wx' })
    const child = Bun.spawn(
      ['sh', file, token, ca, ...(options.verifyOnly ? ['--verify-only'] : (options.args ?? []))],
      {
        stdin: 'inherit',
        stdout: 'inherit',
        stderr: 'inherit',
      },
    )
    if ((await child.exited) !== 0) throw new Error('Verified node bootstrap failed')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}
