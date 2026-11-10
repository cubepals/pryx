// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The pryx binary: operator commands and public sandbox workflows. */
import { PryxError } from '@cubepals/pryx'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { readClientConfig } from './client-config.ts'
import { doctor } from './doctor.ts'
import { key } from './key.ts'
import { node } from './node.ts'
import { project } from './project.ts'
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
      : group === 'doctor'
        ? await doctor(args)
        : group === 'webhook'
          ? await (await import('./webhook.ts')).webhook(args)
          : group === 'templates'
            ? await (await import('./template.ts')).template(['ls'])
            : group === 'template'
              ? await (await import('./template.ts')).template(args)
              : group === 'mcp'
                ? await (await import('./mcp.ts')).mcp(args)
                : group === 'up'
                  ? await up(args)
                  : group === 'node'
                    ? await node(args)
                    : group === 'key'
                      ? await key(args)
                      : group === 'project'
                        ? await project(args)
                        : group === 'sandbox'
                          ? await sandbox(args)
                          : group === 'run'
                            ? await run(args)
                            : (() => {
                                throw new Error('usage: pryx up|node|key|project|sandbox|run|mcp|template')
                              })()
  if (result !== undefined) console.log(JSON.stringify(result, null, 2))
} catch (error) {
  console.error(error instanceof Error ? error.message : 'Command failed')
  if (error instanceof PryxError && error.details !== undefined) console.error(JSON.stringify(error.details))
  process.exitCode = 1
}
