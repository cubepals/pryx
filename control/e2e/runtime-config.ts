// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The single-node daemon configuration fixture, parameterized by its deployment. */

import { join } from 'node:path'

export const nodeConfigFor = (DEPLOYMENT: string) =>
  function nodeConfig(at: {
    home: string
    endpointUrl: string
    dir: string
    apiPort: number
    opsPort: number
    allowedImages?: string[]
    tier?: import('../src/isolation.ts').Isolation
    agent?: boolean
    storeLease?: boolean
    low: number
  }): string {
    const { home, endpointUrl, dir, apiPort, opsPort, low } = at
    return `deployment_id = "${DEPLOYMENT}"
runtime = "${at.tier ?? 'container'}"
state_dir = "${join(home, 'state')}"
[api]
listen = "127.0.0.1:${apiPort}"
[ops]
listen = "127.0.0.1:${opsPort}"
[network]
port_range = [${low}, ${low + (process.env.PRYX_TIER_BENCH === '1' ? 255 : 49)}]
port_quarantine_seconds = 0
[wasm]
distribution = ${JSON.stringify(process.env.PRYX_AGENTOS_DISTRIBUTION ?? '/usr/local/lib/pryx/agentos')}
[workloads]
${at.agent ? '' : 'agent_binary = "/nonexistent/legacy-node-agent"'}
allowed_images = ${JSON.stringify(at.allowedImages ?? ['alpine:'])}
min_memory_mb = 64
[capacity]
reserved_memory_mb = 0
allocatable_memory_mb = 4096
min_free_disk_mb = 256
[transfer]
max_put_mb = 1
${at.storeLease ? '[store_lease]\nenabled = true\nnon_suspending_host = true\n' : ''}
[fleet]
url = "${endpointUrl}"
ca = "${join(dir, 'ca.pem')}"
enrollment_token_file = "${join(home, 'token')}"
heartbeat_seconds = 1
`
  }
