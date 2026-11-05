// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The pryx binary: operator commands and public sandbox workflows. */
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { readClientConfig } from './client-config.ts'
import { key } from './key.ts'
import { node } from './node.ts'
import { run, sandbox } from './sandbox.ts'
import { up } from './up.ts'

if (process.argv[2] === '--version') {
  console.log(`pryx ${PRYX_VERSION}`)
  process.exit(0)
}
const [group, ...args] = process.argv.slice(2)
try {
  const result =
    group === 'config' && args[0] === 'key'
      ? (() => {
          const saved = readClientConfig()
          if (!saved) throw new Error('No CLI credentials')
          console.log(saved.key)
        })()
      : group === 'up'
        ? await up(args)
        : group === 'node'
          ? await node(args)
          : group === 'key'
            ? await key(args)
            : group === 'sandbox'
              ? await sandbox(args)
              : group === 'run'
                ? await run(args)
                : (() => {
                    throw new Error('usage: pryx up|node|key|sandbox|run')
                  })()
  if (result !== undefined) console.log(JSON.stringify(result, null, 2))
} catch (error) {
  console.error(error instanceof Error ? error.message : 'Command failed')
  process.exitCode = 1
}
