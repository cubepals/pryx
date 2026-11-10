// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real non-root image acceptance: shared identity is reported and a stronger requirement is
 * refused. Host provisioning and throughput measurements belong to the installer and collectors. */
import { StatusCodes } from 'http-status-codes'
import type { components } from '../../sdk/typescript/src/generated/api.ts'
import type { MakeRequest } from '../../sdk/typescript/src/transport.ts'
import { SECOND_MS } from '../src/durations.ts'
import { GUARANTEES } from '../src/guarantees.ts'

export async function sharedUidAcceptance(
  image: string,
  command: (args: string[]) => Promise<string>,
  initialKeys: (logs: string) => string[],
) {
  const name = `pryx-ci-shared-uid-${process.pid}`
  await command([
    'docker',
    'run',
    '-d',
    '--name',
    name,
    '--user',
    '1001:1001',
    '-p',
    '127.0.0.1::8080',
    image,
  ])
  try {
    const mapped = (await command(['docker', 'port', name, '8080/tcp'])).trim()
    if (!mapped.startsWith('127.0.0.1:')) throw new Error('Shared-uid fixture must bind loopback')
    const origin = `http://${mapped}`
    const deadline = Date.now() + 30 * SECOND_MS
    let ready = false
    while (Date.now() < deadline) {
      const response = await fetch(`${origin}/healthz`).catch(() => undefined)
      if (response?.ok) {
        const health = (await response.json()) as Record<string, unknown>
        if (health.isolation !== 'process' || health.sharedUid !== true)
          throw new Error('Wrong shared-uid tier')
        ready = true
        break
      }
      await Bun.sleep(SECOND_MS / 100)
    }
    if (!ready) throw new Error('Shared-uid image did not become healthy')
    const keys = initialKeys(await command(['docker', 'logs', name]))
    if (keys.length !== 1 || !keys[0]) throw new Error('Shared-uid initial key differs')
    const create = (body: MakeRequest) =>
      fetch(`${origin}/v1/sandboxes`, {
        method: 'POST',
        headers: { authorization: `Bearer ${keys[0]}`, 'content-type': 'application/json' },
        body: JSON.stringify({ isolation: 'process', start: false, ...body }),
      })
    const refused = await create({ require: ['separate-users'] })
    const error = (await refused.json()) as components['schemas']['Error']
    if (
      refused.status !== StatusCodes.SERVICE_UNAVAILABLE ||
      error.error?.code !== 'no_room' ||
      !error.error.message.includes('separate-users')
    )
      throw new Error('Shared-uid requirement was not refused by name')
    const created = await create({})
    if (created.status !== StatusCodes.CREATED) throw new Error('Ordinary shared-uid creation failed')
    const sandbox = (await created.json()) as components['schemas']['Sandbox']
    if (!sandbox.guarantees) throw new Error('Shared-uid sandbox omitted its guarantees')
    for (const guarantee of GUARANTEES)
      if (sandbox.guarantees[guarantee] !== (guarantee === 'memory-sleep'))
        throw new Error(`Shared-uid guarantee differs: ${guarantee}`)
    console.log(
      JSON.stringify({ sharedUid: true, requirementsRefused: true, guarantees: sandbox.guarantees }),
    )
  } catch (error) {
    let logs = await command(['docker', 'logs', name])
    for (const key of initialKeys(logs)) logs = logs.replaceAll(key, '[redacted]')
    console.error(logs.slice(-8192))
    throw error
  } finally {
    await command(['docker', 'rm', '-fv', name])
  }
}
