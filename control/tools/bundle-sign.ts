// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sign an already built host archive with the release statement's distinct domain. */
import { createPrivateKey, sign } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { hashFile } from '../../cli/src/bundle-download.ts'
import { bundleStatement, HOST_BUNDLE } from '../src/bundle-release.ts'
import { PRYX_VERSION } from '../src/generated/release.ts'

const [directory, privateKey] = process.argv.slice(2)
if (!directory || !privateKey) throw new Error('bundle-sign.ts <artifact-directory> <private-key-file>')
const digest = await hashFile(join(directory, HOST_BUNDLE))
writeFileSync(
  join(directory, `${HOST_BUNDLE}.sig`),
  sign(null, Buffer.from(bundleStatement(PRYX_VERSION, digest)), createPrivateKey(readFileSync(privateKey))),
)
