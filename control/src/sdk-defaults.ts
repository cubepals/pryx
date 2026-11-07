// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public SDK convenience defaults. Raw make requests retain image entrypoints. */

import agent from '../../agent/openapi.json'
import node from '../../openapi/node-reads.openapi.json'
import { DAY_S, HOUR_S, MINUTE_S, SECOND_MS } from './durations.ts'
export const SDK_DEFAULTS = {
  stdinBytes: node['x-pryx-agent'].maxRequestBytes,
  guestFrameBytes: node['x-pryx-wasm'].maxHttpFrameBytes,
  archiveBytes: node['x-pryx-agent'].archiveBytes,
  archiveEntries: node['x-pryx-agent'].archiveEntries,
  agentVersion: agent['x-pryx-agent-version'],
  secondMs: SECOND_MS,
  captureBytes: node['x-pryx-agent'].captureBytes,
  tokenRenewBeforeSeconds: node['x-pryx-agent'].tokenClockSkewSeconds,
  runTimeoutSeconds: node.components.schemas.ExecRequest.properties.timeoutSeconds.default,
  sparePorts: 3,
  units: { s: 1, m: MINUTE_S, h: HOUR_S, d: DAY_S },
  fileChunkBytes: 32 * 1024,
  entrypoint: [
    'sh',
    '-c',
    'trap \'kill "$child"; exit 0\' TERM INT; tail -f /dev/null & child=$!; wait "$child"',
  ],
}
