// SPDX-License-Identifier: FSL-1.1-ALv2
/** Interpret opt-in requests after the stock CLI has authenticated this release archive.
 * The stock preset still owns incoming data, service startup and the readiness signal.
 */
import { lstatSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

export default async function dispatch(files, command, args, stock) {
  function invoke(args, archive = files.archive, env = process.env) {
    const child = Bun.spawnSync(['python3', '-I', archive, ...args], { env, stdout: 'pipe', stderr: 'pipe' })
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
  // Server and image provisioning permanently drop to the sudo caller to save their
  // credentials. Keep only the already authenticated, bounded bytes across that boundary.
  let retained
  if (command === 'up' && process.getuid() === 0 && process.env.SUDO_UID && process.env.SUDO_UID !== '0') {
    retained = { archive: readFileSync(files.archive), signature: readFileSync(files.signature) }
    files.close()
  }
  let active = false
  function activate() {
    if (active) return
    if (retained) {
      const uid = process.getuid()
      const account = Bun.spawnSync(['getent', 'passwd', String(uid)], { stdout: 'pipe', stderr: 'ignore' })
      const fields = account.stdout.toString().trim().split(':')
      const homedir = fields[5]
      if (account.exitCode || fields[2] !== String(uid) || !homedir?.startsWith('/'))
        throw new Error('Could not resolve the credential owner for experimental activation')
      const info = lstatSync(homedir)
      if (!info.isDirectory() || info.uid !== uid || info.mode & 0o022)
        throw new Error('The credential owner needs an owned, protected home for experimental activation')
      const scratch = files.privateStage()
      try {
        const archive = join(scratch, 'lab.pyz'), signature = join(scratch, 'signature')
        writeFileSync(archive, retained.archive, { flag: 'wx', mode: 0o600 })
        writeFileSync(signature, retained.signature, { flag: 'wx', mode: 0o600 })
        invoke(['activate', signature, ...selected], archive, { ...process.env, HOME: homedir })
      } finally {
        rmSync(scratch, { recursive: true, force: true })
        retained = undefined
      }
    } else {
      invoke(['activate', files.signature, ...selected])
      files.close()
    }
    active = true
  }
  const result = command === 'up' ? await stock(base, activate) : undefined
  activate()
  return result
}
