// SPDX-License-Identifier: FSL-1.1-ALv2
/** Release image acceptance under recorded hosting envelopes, not a provider-kernel emulator.
 * Starts without added privileges and keeps one volume across a restart; runs every UI caller. */

import { randomBytes } from 'node:crypto'
import { appendFileSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { WELCOME_DETAILS_PATH } from '../src/welcome-path.ts'

async function command(args: string[], env = process.env) {
  const child = Bun.spawn(args, { env, stdout: 'pipe', stderr: 'pipe' })
  const [stdout, errors] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  if (await child.exited) {
    const redacted = environmentSecrets(errors, env)
    throw new Error(`Hosting command failed: ${args.slice(0, 2).join(' ')}: ${redacted.slice(-2048)}`)
  }
  return stdout
}
function environmentSecrets(value: string, env: NodeJS.ProcessEnv) {
  for (const name of ['PRYX_API_KEY', 'PRYX_BOOTSTRAP_API_KEY']) {
    const key = env[name]
    if (key) value = value.replaceAll(key, '[redacted]')
  }
  return value
}
const temporary = mkdtempSync(join(tmpdir(), 'pryx-hosting-image-'))
const image = process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test'
const cli = join(temporary, 'pryx')
const fixture = (await command(['docker', 'create', image])).trim()
try {
  await command(['docker', 'cp', `${fixture}:/usr/local/bin/pryx`, cli])
} finally {
  await command(['docker', 'rm', '-v', fixture])
}
const summaries: string[] = []
try {
  for (const platform of ['render', 'railway', 'fly', 'koyeb', 'coolify', 'ordinary-user']) {
    const name = `pryx-hosting-${platform}-${process.pid}`
    const volume = `${name}-state`
    const supplied = ['render', 'railway', 'coolify'].includes(platform)
    const key = `pryx_${randomBytes(32).toString('base64url')}`
    const port = 18080
    const internalPort = platform === 'render' ? 10000 : platform === 'ordinary-user' ? 9080 : 8080
    const origin = `http://127.0.0.1:${port}`
    const environment = { ...process.env, PRYX_BOOTSTRAP_API_KEY: key }
    const platformOrigin =
      platform === 'render' ? ['-e', `RENDER_EXTERNAL_URL=${origin}`] : ['-e', `PRYX_PUBLIC_URL=${origin}`]
    try {
      await command(
        [
          'docker',
          'run',
          '-d',
          '--name',
          name,
          '--memory',
          '2g',
          '--security-opt',
          'no-new-privileges',
          ...(platform === 'ordinary-user' ? ['--user', '1000:1000'] : []),
          '-p',
          `127.0.0.1:${port}:${internalPort}`,
          '--mount',
          `type=volume,source=${volume},target=/var/lib/pryx`,
          '-e',
          `PORT=${internalPort}`,
          '-e',
          `PRYX_HOSTING_PLATFORM=${platform}`,
          ...(supplied ? ['-e', 'PRYX_BOOTSTRAP_API_KEY'] : []),
          ...platformOrigin,
          image,
        ],
        environment,
      )
      const healthy = async () => {
        const deadline = Date.now() + 2 * MINUTE_MS
        while (Date.now() < deadline) {
          if ((await fetch(`${origin}/healthz`).catch(() => undefined))?.ok) return
          await Bun.sleep(SECOND_MS)
        }
        // Capture private diagnostics without printing a credential-bearing service log.
        throw new Error(
          `${platform} image did not become healthy; inspect the runner container before cleanup`,
        )
      }
      await healthy()
      let apiKey = key
      const readKeys = async () =>
        (await command(['docker', 'logs', name])).split('\n').flatMap((line) => {
          try {
            const parsed = JSON.parse(line)
            return typeof parsed.apiKey === 'string' ? [parsed.apiKey as string] : []
          } catch {
            return []
          }
        })
      const keys = await readKeys()
      if (supplied && (keys.length || (await command(['docker', 'logs', name])).includes(key)))
        throw new Error('Platform key printed to service log')
      if (!supplied) {
        if (keys.length !== 1 || !keys[0]) throw new Error('Generated key not printed once')
        apiKey = keys[0]
      }
      const details = await fetch(`${origin}${WELCOME_DETAILS_PATH}`, {
        headers: { authorization: `Bearer ${apiKey}` },
      })
      if (!details.ok) throw new Error(`${platform} welcome details returned HTTP ${details.status}`)
      await command([process.execPath, 'control/tools/hosting-callers.ts'], {
        ...process.env,
        PRYX_API_URL: origin,
        PRYX_API_KEY: apiKey,
        PRYX_HOSTING_CLI: resolve(cli),
      })
      await command(['docker', 'restart', name])
      await healthy()
      const retained = await fetch(`${origin}/v1/project`, { headers: { authorization: `Bearer ${apiKey}` } })
      if (!retained.ok || (await readKeys()).length !== keys.length)
        throw new Error('Restart changed or repeated the first key')
      const summary = `${platform}: ordinary Docker envelope; rootfs writable; PORT=${internalPort}; one volume; no added privilege; welcome/auth, six callers, restart: pass`
      summaries.push(summary)
      console.log(summary)
    } finally {
      await command(['docker', 'rm', '-f', name])
      await command(['docker', 'volume', 'rm', volume])
    }
  }
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
if (process.env.GITHUB_STEP_SUMMARY)
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summaries.join('\n')}\n`)
