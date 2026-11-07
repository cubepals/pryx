// SPDX-License-Identifier: FSL-1.1-ALv2
/** Prepare the release-only bootstrap signing statement from the exact published daemon and agent. */
import { readFileSync, writeFileSync } from 'node:fs'
import { PRYX_VERSION } from '../src/generated/release.ts'
import { bootstrapHash, bootstrapStatement, joinBootstrapScript } from '../src/join-bootstrap.ts'

const [daemon, agent, output] = process.argv.slice(2)
if (!daemon || !agent || !output) throw new Error('join-bootstrap.ts daemon agent statement-output')
const script = joinBootstrapScript(
  readFileSync(new URL('../src/join.sh', import.meta.url), 'utf8'),
  readFileSync(new URL('../src/daemon.json', import.meta.url), 'utf8'),
  bootstrapHash(readFileSync(daemon)),
  bootstrapHash(readFileSync(agent)),
)
writeFileSync(output, bootstrapStatement(PRYX_VERSION, bootstrapHash(script)))
