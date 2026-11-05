// SPDX-License-Identifier: FSL-1.1-ALv2
/** Apply the prepared native backend to the pinned public release and build its executable.
 * Reference checkouts stay read-only; the caller owns the new output directory. */
import { appendFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const output = resolve(process.argv[2] ?? 'target/prepared-celld')
const commit = '8702f6d186c77698dbfb30cf29e3b1922a88f008'
const source = join(output, 'source')
mkdirSync(output, { recursive: true })
async function run(args: string[], cwd = output, env = process.env) {
  const child = Bun.spawn(args, { cwd, env, stdout: 'inherit', stderr: 'inherit' })
  if (await child.exited) throw new Error(`${args[0]} failed`)
}
if (existsSync(source)) throw new Error('Choose a fresh output directory for the public celld patch')
const local =
  process.env.PRYX_PATCH_SOURCE_ROOT && join(process.env.PRYX_PATCH_SOURCE_ROOT, 'reference-celld')
if (local && existsSync(local)) await run(['git', 'clone', '--shared', '--no-checkout', local, source])
else {
  await run(['git', 'init', source])
  await run(['git', '-C', source, 'remote', 'add', 'origin', 'https://github.com/denoland/celld'])
  await run(['git', '-C', source, 'fetch', '--depth=1', 'origin', commit])
}
await run(['git', '-C', source, 'checkout', '--detach', commit])
const patch = join(import.meta.dir, 'change.patch')
await run(['git', '-C', source, 'apply', '--check', patch])
await run(['git', '-C', source, 'apply', patch])
const target = join(output, 'target')
await run(['cargo', 'build', '--locked', '--bin', 'celld'], source, {
  ...process.env,
  CARGO_TARGET_DIR: target,
  CARGO_PROFILE_DEV_DEBUG: '0',
  CARGO_INCREMENTAL: '0',
})
const binary = join(target, 'debug', 'celld')
await run([binary, '--version'])
writeFileSync(
  join(output, 'result.json'),
  JSON.stringify({ commit, binary, profile: 'dev, debug=0, incremental=0', source }, null, 2),
)
if (process.env.GITHUB_ENV)
  appendFileSync(process.env.GITHUB_ENV, `CELLD_NATIVE_BIN=${binary}\nCELLD_NATIVE_SOURCE=${source}\n`)
