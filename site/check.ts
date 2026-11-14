// SPDX-License-Identifier: FSL-1.1-ALv2
/** One-command site setup/check. Dependencies stay in this checkout; lifecycle scripts stay off. */
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const cwd = dirname(fileURLToPath(import.meta.url))
for (const command of [
  ['bun', 'install', '--frozen-lockfile', '--ignore-scripts', '--cache-dir', 'node_modules/.cache/bun'],
  ['bun', 'tools/build.ts'],
  ['bun', join(cwd, '../node_modules/typescript/bin/tsc'), '--noEmit', '-p', 'tsconfig.json'],
]) {
  const child = Bun.spawn(command, { cwd, stdout: 'inherit', stderr: 'inherit' })
  if (await child.exited) throw new Error('Site check failed')
}
