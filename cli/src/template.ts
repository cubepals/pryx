// SPDX-License-Identifier: FSL-1.1-ALv2
/** Built-in template creation delegates preparation and per-node warming to the public API. */
import { SANDBOX_AGENT_TEMPLATE, sandboxAgentTemplate } from '../../integrations/rivet/src/template.ts'
import { option, publicTransport } from './sandbox.ts'

export async function template(args: string[]) {
  if (args[0] === 'ls') return publicTransport().request('/v1/templates')
  if (args[0] !== 'create' || args[1] !== SANDBOX_AGENT_TEMPLATE)
    throw new Error('usage: pryx template create sandbox-agent [--image reference] | template ls')
  const result = await publicTransport().request<{ id: string; copies: { ready: boolean }[] }>(
    '/v1/templates',
    'POST',
    {
      name: SANDBOX_AGENT_TEMPLATE,
      spec: sandboxAgentTemplate(option(args, '--image')),
    },
  )
  if (!result.copies.length || result.copies.some((copy) => !copy.ready))
    throw new Error('Template is registered but not warm on every node; inspect pryx template ls')
  return result
}
