// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real coexistence in the ordinary image with the host engine socket. Matching bind paths and
 * host networking connect that engine's published listeners; no extra capability or profile is used.
 * The established tier collectors remain responsible for latency and memory budgets. */
import { chmodSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import { privateDirectory } from '../../deploy/local-layout.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { freePort, until } from '../e2e/waits.ts'
import { SECOND_MS } from '../src/durations.ts'
import { GUARANTEES, type Guarantee } from '../src/guarantees.ts'
import { ISOLATIONS } from '../src/isolation.ts'
import { freshDatabase } from '../test/nodes.ts'

type Command = (args: string[]) => Promise<string>
export async function mixedTierAcceptance(
  image: string,
  command: Command,
  initialKeys: (logs: string) => string[],
) {
  const name = `pryx-ci-mixed-${process.pid}`
  const root = mkdtempSync(join(tmpdir(), 'pryx-ci-mixed-'))
  chmodSync(root, 0o755)
  const port = await freePort(),
    origin = `http://127.0.0.1:${port}`
  const held: Awaited<ReturnType<Pryx['sandbox']>>[] = []
  const database = await freshDatabase()
  const storage = {
    DATABASE_URL: database.url,
    S3_ENDPOINT: process.env.S3_TEST_ENDPOINT,
    S3_BUCKET: name,
    S3_ACCESS_KEY: process.env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
    S3_SECRET_KEY: process.env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
  }
  try {
    if (!storage.S3_ENDPOINT) throw new Error('S3_TEST_ENDPOINT is required for mixed-tier acceptance')
    await command([
      'docker',
      'run',
      '-d',
      '--name',
      name,
      '--network',
      'host',
      '--mount',
      `type=bind,src=${root},dst=${root}`,
      '--mount',
      'type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock',
      '-e',
      `PRYX_STATE_DIR=${root}`,
      '-e',
      `PORT=${port}`,
      ...Object.entries(storage).flatMap(([key, value]) => ['-e', `${key}=${value}`]),
      image,
    ])
    await until(
      'mixed-tier image',
      async () => (await fetch(`${origin}/healthz`).catch(() => undefined))?.ok,
      60 * SECOND_MS,
    )
    const host = JSON.parse(await command(['docker', 'inspect', '--format', '{{json .HostConfig}}', name]))
    if (host.Privileged || host.CapAdd?.length || host.SecurityOpt?.length)
      throw new Error('Mixed-tier image acquired a privilege or profile override')
    const keys = initialKeys(await command(['docker', 'logs', name]))
    if (keys.length !== 1 || !keys[0]) throw new Error('Mixed-tier image initial key differs')
    const sdk = new Pryx({ url: origin, key: keys[0] })
    const expected: Record<string, Guarantee[]> = {
      container: ['memory-limit', 'cpu-limit', 'private-network', 'private-pids', 'memory-sleep'],
      process: ['separate-users', 'memory-sleep'],
      wasm: [],
    }
    for (const isolation of ISOLATIONS) {
      const box = await sdk.sandbox({
        isolation,
        ...(isolation === 'container' ? { image: 'alpine:3.22' } : {}),
      })
      held.push(box)
      if (box.isolation !== isolation) throw new Error(`Requested ${isolation}, received ${box.isolation}`)
      for (const guarantee of GUARANTEES)
        if (box.guarantees?.[guarantee] !== expected[isolation]?.includes(guarantee))
          throw new Error(`${isolation} guarantee differs: ${guarantee}`)
      const output = await box.run(['sh', '-c', 'printf mixed'])
      if (output.code !== 0 || output.stdout !== 'mixed') throw new Error(`${isolation} command differs`)
      await box.write('mixed-tier-proof', 'saved')
      if ((await box.read('mixed-tier-proof')) !== 'saved') throw new Error(`${isolation} data differs`)
    }
    const unnamed = await sdk.sandbox({ image: 'alpine:3.22' })
    held.push(unnamed)
    if (unnamed.isolation !== 'container')
      throw new Error('Open tier did not select the preferred healthy tier')
    const alive = await sdk.sandboxes()
    for (const box of held)
      if (
        !alive.some(
          (view) => view.id === box.id && view.isolation === box.isolation && view.state === 'running',
        )
      )
        throw new Error('Mixed tiers did not remain running side by side')
    const cli = JSON.parse(await command(['docker', 'exec', name, 'pryx', 'doctor', '--json']))
    for (const tier of ISOLATIONS)
      if (cli.tiers?.[tier]?.state !== 'on') throw new Error(`CLI doctor omitted available ${tier}`)
    const fixed = join(root, 'doctor-fixed')
    await command([
      'docker',
      'exec',
      name,
      'python3',
      '-c',
      'from pathlib import Path; import sys; source=Path(sys.argv[1]); destination=Path(sys.argv[2]); destination.parent.mkdir(mode=0o700); text=source.read_text(); assert \'runtime = "auto"\' in text; destination.write_text(text.replace(\'runtime = "auto"\', \'runtime = "process"\', 1)); destination.chmod(0o600)',
      join(privateDirectory(root), IMAGE_NODE_CONFIG),
      join(fixed, IMAGE_NODE_CONFIG),
    ])
    const single = JSON.parse(
      await command([
        'docker',
        'exec',
        '-e',
        `PRYX_LOCAL_CONFIG_DIR=${fixed}`,
        name,
        'pryx',
        'doctor',
        '--json',
      ]),
    )
    if (
      single.tiers?.process?.state !== 'on' ||
      single.tiers?.container?.state !== 'off' ||
      single.tiers?.wasm?.state !== 'off'
    )
      throw new Error('CLI doctor did not respect the explicit process configuration')
    const doctor = JSON.parse(
      await command([
        'docker',
        'exec',
        name,
        '/pryxd',
        'doctor',
        '--config',
        join(privateDirectory(root), IMAGE_NODE_CONFIG),
        '--preflight',
        '--json',
      ]),
    )
    for (const tier of ISOLATIONS)
      if (
        !doctor.checks.some(
          (check: { name: string; status: string }) =>
            check.name === `tier.${tier}` && check.status === 'pass',
        )
      )
        throw new Error(`Daemon doctor omitted available ${tier}`)
    for (const box of held) await box.delete()
    held.length = 0
    console.log(
      JSON.stringify({
        mixedTiers: ISOLATIONS,
        sideBySide: true,
        guarantees: expected,
        doctors: true,
        unnamed: 'container',
      }),
    )
  } catch (error) {
    let logs = await command(['docker', 'logs', name])
    for (const key of initialKeys(logs)) logs = logs.replaceAll(key, '[redacted]')
    console.error(logs.slice(-8192))
    throw error
  } finally {
    const running = (await command(['docker', 'ps', '-aq', '--filter', `name=^${name}$`])).trim()
    if (running) await command(['docker', 'rm', '-fv', name])
    // A workload lives in the host engine. Cleanup claims only bind sources beneath this fixture.
    const ids = (await command(['docker', 'ps', '-aq', '--filter', 'label=blocklyd.deployment=pryx']))
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    for (const id of ids) {
      const mounts = JSON.parse(await command(['docker', 'inspect', '--format', '{{json .Mounts}}', id])) as {
        Source: string
      }[]
      if (mounts.some((mount) => mount.Source.startsWith(`${root}/`)))
        await command(['docker', 'rm', '-f', id])
    }
    await command(['sudo', 'rm', '-rf', '--', root])
    await database.drop()
  }
}
