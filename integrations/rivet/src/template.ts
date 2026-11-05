// SPDX-License-Identifier: FSL-1.1-ALv2
/** The bundled sandbox-agent template profile. Private server ingress follows the current
 * upstream Docker deployment contract; no coding-agent provider credentials enter the image. */
import type { MakeRequest } from '@cubepals/pryx'
import metadata from '../package.json' with { type: 'json' }

// sandbox-agent 0.5.5's deploy/docker documentation and Docker provider default.
export const SANDBOX_AGENT_PORT = 3000
export const SANDBOX_AGENT_TEMPLATE = 'sandbox-agent'
export const SANDBOX_AGENT_IMAGE = `ghcr.io/cubepals/pryx-sandbox-agent:${metadata.version}`
export function sandboxAgentTemplate(image = SANDBOX_AGENT_IMAGE): MakeRequest {
  return {
    image,
    isolation: 'container',
    env: {
      HOME: '/data',
      XDG_DATA_HOME: '/data/.local/share',
      XDG_STATE_HOME: '/data/.local/state',
      XDG_CACHE_HOME: '/data/.cache',
    },
    entrypoint: [
      'sandbox-agent',
      'server',
      '--no-token',
      '--host',
      '0.0.0.0',
      '--port',
      String(SANDBOX_AGENT_PORT),
    ],
    ports: [{ name: 'agent', port: SANDBOX_AGENT_PORT, protocol: 'tcp', audience: 'private' }],
    start: true,
  }
}
