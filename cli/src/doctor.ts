// SPDX-License-Identifier: FSL-1.1-ALv2
/** Display the daemon's measured host document, using the same executable and caller permissions. */
import { existsSync } from 'node:fs'

export async function doctor(args: string[]) {
  if (args.some((argument) => argument !== '--json')) throw new Error('usage: pryx doctor [--json]')
  const binary =
    process.env.PRYX_PRYXD_BIN ??
    Bun.which('pryxd') ??
    ['/opt/pryx/pryxd', '/usr/local/bin/pryxd'].find((path) => existsSync(path))
  if (!binary) throw new Error('pryxd is missing; install pryx on this host before running pryx doctor')
  const child = Bun.spawn([binary, 'probe', ...(args.includes('--json') ? ['--json'] : [])], {
    stdin: 'ignore',
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (await child.exited) throw new Error('The host probe failed; inspect the reported requirement')
}
