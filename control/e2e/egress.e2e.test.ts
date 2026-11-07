// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host firewall acceptance uses the real container bridge and root fixture. Virtual agent
 * acceptance stays unprivileged in its separate invocation; this case remains in the full suite. */
import { expect, test } from 'bun:test'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'

test(
  'container hostname egress admits one host and refuses another on the bridge',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('container', { egressPolicy: true })
    let box: Sandbox | undefined
    try {
      box = await c.sandbox({
        image: 'alpine:pryx-tls',
        egress: { allow: ['example.com'], deny: ['example.org'] },
        resources: { memoryMb: 256 },
      })
      expect(
        (
          await box.run([
            'curl',
            '--fail',
            '--silent',
            '--show-error',
            '--max-time',
            '20',
            'https://example.com',
          ])
        ).code,
      ).toBe(0)
      for (const url of ['https://example.org', 'http://example.org', 'http://1.1.1.1'])
        expect(
          (
            await box.run(['curl', '--fail', '--silent', '--show-error', '--max-time', '5', url], {
              env: { HTTP_PROXY: '', HTTPS_PROXY: '', ALL_PROXY: '' },
            })
          ).code,
        ).not.toBe(0)
      await box.network('none')
      expect((await box.run(['curl', '--fail', '--max-time', '3', 'https://example.com'])).code).not.toBe(0)
      await box.network('internet')
      expect(
        (await box.run(['curl', '--fail', '--silent', '--max-time', '20', 'https://example.com'])).code,
      ).toBe(0)
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box?.delete()
      await c.close()
    }
  },
  3 * MINUTE_MS,
)
