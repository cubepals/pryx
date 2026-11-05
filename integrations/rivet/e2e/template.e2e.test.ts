// SPDX-License-Identifier: FSL-1.1-ALv2
/** The current sandbox-agent SDK provisions the repository's warmed template by provider name.
 * Its server performs a real command; no external coding-agent API credential is required. */
import { expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { SandboxAgent } from 'sandbox-agent'
import { deployment, ready } from '../../../control/e2e/deployment.ts'
import { MINUTE_MS } from '../../../control/src/durations.ts'
import type { pryxProvider } from '../src/agentos.ts'

test(
  'Rivet sandbox-agent runs a command from the warmed pryx template',
  async () => {
    if (!ready) throw new Error('Template acceptance requires the real CI deployment')
    const image = process.env.PRYX_SANDBOX_AGENT_IMAGE
    if (!image) throw new Error('The repository sandbox-agent image must be built first')
    const d = await deployment({ agent: true, allowedImages: [image] })
    const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
    try {
      const create = Bun.spawn(
        [process.execPath, 'cli/src/main.ts', 'template', 'create', 'sandbox-agent', '--image', image],
        { env: d.env, stdout: 'pipe', stderr: 'inherit' },
      )
      const output = await new Response(create.stdout).text()
      expect(await create.exited).toBe(0)
      const template = JSON.parse(output) as { copies: { ready: boolean }[] }
      expect(template.copies.length).toBeGreaterThan(0)
      expect(template.copies.every((copy) => copy.ready)).toBe(true)
      if (!process.env.PRYX_PATCH_PROVIDER) throw new Error('The public patched provider must be built first')
      const patched = (await import(process.env.PRYX_PATCH_PROVIDER)) as { pryx: typeof pryxProvider }
      const provider = patched.pryx(sdk, { key: crypto.randomUUID(), template: 'sandbox-agent' })
      expect(provider.name).toBe('pryx')
      const remote = await SandboxAgent.start({ sandbox: provider })
      try {
        const result = await remote.runProcess({
          command: 'sh',
          args: ['-c', 'printf warmed-on-pryx'],
          cwd: '/data',
        })
        expect(result.exitCode).toBe(0)
        expect(result.stdout).toBe('warmed-on-pryx')
      } finally {
        await remote.destroySandbox()
      }
    } finally {
      await d.close()
    }
  },
  5 * MINUTE_MS,
)
