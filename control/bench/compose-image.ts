// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise the shipped Compose service graph with the already-built release image. The CI
 * Docker build separately proves its identical build context; this avoids compiling it twice. */
import { resolve } from 'node:path'
import { SECOND_MS } from '../src/durations.ts'
import { PRYX_VERSION } from '../src/generated/release.ts'

const project = `pryx-ci-compose-${process.pid}`
const prefix = ['docker', 'compose', '-p', project, '-f', resolve('docker-compose.yml')]
async function command(args: string[], env = process.env) {
  const child = Bun.spawn(args, { env, stdout: 'pipe', stderr: 'pipe' })
  const [stdout] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()])
  if (await child.exited) throw new Error(`${args[0]} failed during Compose acceptance`)
  return stdout
}
async function healthy() {
  const end = Date.now() + 30 * SECOND_MS
  while (Date.now() < end) {
    if ((await fetch('http://127.0.0.1:8080/healthz').catch(() => undefined))?.ok) return
    await Bun.sleep(SECOND_MS / 100)
  }
  throw new Error('Compose service did not become healthy')
}
function initialKeys(logs: string) {
  return logs.split('\n').flatMap((line) => {
    try {
      const value = JSON.parse(line)
      return typeof value.apiKey === 'string' ? [value.apiKey as string] : []
    } catch {
      return []
    }
  })
}
async function hello(key: string) {
  const output = await command([process.execPath, 'cli/src/main.ts', 'run', '--', 'echo', 'hello'], {
    ...process.env,
    PRYX_API_URL: 'http://127.0.0.1:8080',
    PRYX_API_KEY: key,
  })
  if (output.trim() !== 'hello') throw new Error('Compose CLI hello differs')
}
try {
  await command([
    'docker',
    'tag',
    process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test',
    `ghcr.io/cubepals/pryx:${PRYX_VERSION}`,
  ])
  const probe = JSON.parse(
    await command([
      'docker',
      'run',
      '--rm',
      '--entrypoint',
      '/pryxd',
      process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test',
      'probe',
      '--json',
    ]),
  ) as { landlockAbi: number; capabilities: Record<string, { state: string }> }
  for (const name of [
    'kvm',
    'kvm-device',
    'fuse-device',
    'docker',
    'docker-user',
    'podman',
    'podman-user',
    'systemd',
  ])
    if (probe.capabilities[name]?.state !== 'off')
      throw new Error(`Ordinary image probe: ${name} must be off`)
  for (const name of ['user-namespaces', 'host-firewall'])
    if (probe.capabilities[name]?.state !== 'unknown')
      throw new Error(`Ordinary image probe: ${name} must be unknown`)
  for (const name of ['landlock', 'separate-users', 'cgroup-controllers'])
    if (probe.capabilities[name]?.state !== 'on') throw new Error(`Ordinary image probe: ${name} must be on`)
  if (probe.landlockAbi < 3)
    throw new Error('Ordinary image probe disagrees with its namespace/Landlock policy')
  await command([...prefix, 'config', '--quiet'])
  await command([...prefix, 'up', '-d', '--no-build', '--pull', 'never'])
  await healthy()
  const keys = initialKeys(await command([...prefix, 'logs', '--no-log-prefix', '--no-color', 'pryx']))
  if (keys.length !== 1 || !keys[0]) throw new Error('Compose initial key differs')
  await hello(keys[0])
  await command([...prefix, 'restart'])
  await healthy()
  await hello(keys[0])
  if (initialKeys(await command([...prefix, 'logs', '--no-log-prefix', '--no-color', 'pryx'])).length !== 1)
    throw new Error('Compose restart printed the key again')
  console.log(JSON.stringify({ compose: true, hello: true, restart: true, keyPrintedOnce: true }))
} finally {
  await command([...prefix, 'down', '--volumes', '--remove-orphans'])
}
