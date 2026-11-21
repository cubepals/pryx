// SPDX-License-Identifier: FSL-1.1-ALv2
/** Interpret opt-in requests after the stock CLI has authenticated this release archive.
 * The stock preset still owns incoming data, service startup and the readiness signal.
 */
import { rmSync } from 'node:fs'

export default async function dispatch(files, command, args, stock) {
  function invoke(args) {
    const child = Bun.spawnSync(['python3', '-I', files.archive, ...args], { stdout: 'pipe', stderr: 'pipe' })
    if (child.exitCode !== 0) throw new Error(child.stderr.toString().trim().slice(-512) || 'Experimental lab command failed')
    return child.stdout.toString()
  }

  if (command === 'doctor') {
    const capabilities = JSON.parse(invoke(['capabilities', files.generation, files.signature]))
    if (args.includes('--json')) console.log(JSON.stringify({ ...JSON.parse(stock), experimental: capabilities }))
    else {
      process.stdout.write(stock)
      for (const [name, info] of Object.entries(capabilities))
        console.log(`experimental.${name}: ${info.on ? 'on' : 'off'} (${info.reason})`)
    }
    return
  }
  if (command === 'experimental') {
    if (args[0] === 'run' && args.length === 2) {
      process.stdout.write(invoke(['run', files.generation, files.signature, args[1]]))
      return
    }
    if (args[0] !== 'install') throw new Error('usage: pryx experimental run <feature>')
    args = args.slice(1)
  }
  const delimiter = args.indexOf('--installer-arguments')
  const input = delimiter < 0 ? args : args.slice(delimiter + 1)
  const base = delimiter < 0 ? [] : args.slice(0, delimiter)
  const selected = []
  for (let index = 0; index < input.length; index++) {
    const arg = input[index]
    if (arg === '--feature') selected.push(arg, input[++index] ?? '')
    else if (delimiter < 0 && arg !== '--experimental') base.push(arg)
  }
  if (command === 'experimental' && delimiter < 0 && base.length) throw new Error('Unexpected install arguments')
  invoke(['validate', ...selected])
  let active = false
  function activate() {
    if (active) return
    invoke(['activate', files.signature, ...selected])
    active = true
    // A foreground base can run until shutdown. Its callers no longer need these scratch copies.
    rmSync(files.archive)
    rmSync(files.signature)
  }
  const result = command === 'up' ? await stock(base, activate) : undefined
  activate()
  return result
}
