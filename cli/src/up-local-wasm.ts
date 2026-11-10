// SPDX-License-Identifier: FSL-1.1-ALv2
/** Verify a staged Wasm distribution with a complete throwaway node configuration. TLS
 * fixtures and scratch state stay inside staging and never become the installed identity. */
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { MINUTE_MS } from '../../control/src/durations.ts'

async function command(args: string[], env: NodeJS.ProcessEnv) {
  const child = Bun.spawn(args, { env, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
  const timer = setTimeout(() => child.kill('SIGKILL'), MINUTE_MS)
  try {
    const [stdout, , code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    return { stdout, code }
  } finally {
    clearTimeout(timer)
  }
}
export async function verifyLocalWasm(
  binary: string,
  bundle: string,
  temporary: string,
  env: NodeJS.ProcessEnv,
) {
  const certs = join(temporary, 'preflight-certs')
  mkdirSync(certs, { mode: 0o700 })
  if ((await command([binary, 'dev-certs', '--out', certs], env)).code)
    throw new Error('Could not create temporary certificates for the wasm preflight')
  const config = join(temporary, 'wasm-preflight.toml')
  writeFileSync(
    config,
    `deployment_id = "pryx"
node_id = "preflight"
runtime = "wasm"
state_dir = ${JSON.stringify(join(temporary, 'node'))}
[api]
listen = "127.0.0.1:7443"
[api.tls]
cert = ${JSON.stringify(join(certs, 'node.pem'))}
key = ${JSON.stringify(join(certs, 'node.key'))}
client_ca = ${JSON.stringify(join(certs, 'ca.pem'))}
allowed_clients = ["control-plane.dev.blockly.internal"]
[workloads]
user = "${process.getuid?.() || 1000}:${process.getgid?.() || 1000}"
[wasm]
distribution = ${JSON.stringify(join(bundle, 'agentos'))}
`,
    { mode: 0o600, flag: 'wx' },
  )
  const result = await command([binary, 'doctor', '--json', '--preflight', '--config', config], env)
  if (result.code) {
    let detail = 'runtime doctor did not complete'
    try {
      const report = JSON.parse(result.stdout) as {
        checks: Array<{ status: string; name: string; detail: string; fix?: string }>
      }
      detail = report.checks
        .filter((check) => check.status === 'fail')
        .map((check) => `${check.name}: ${check.detail}; ${check.fix ?? ''}`)
        .join('; ')
    } catch {}
    throw new Error(`The wasm distribution failed preflight: ${detail}`)
  }
}
