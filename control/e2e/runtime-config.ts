// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The single-node daemon configuration fixture, parameterized by its deployment. */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import nodeSchema from '../../openapi/node-reads.openapi.json'

export const TIER_BENCH_UNIT = {
  memoryMb: nodeSchema['x-pryx-resources'].defaultMinMemoryMb,
  cpuMillis: nodeSchema['x-pryx-resources'].minCpuMillis,
}
// Fifty idle sandboxes request five cores on the observed four-core runner.
export const TIER_BENCH_CPU_OVERCOMMIT = 2
export const TIER_BENCH_LARGE_UNITS = 2
export const TIER_BENCH_LARGE_UNIT = {
  memoryMb: TIER_BENCH_UNIT.memoryMb * TIER_BENCH_LARGE_UNITS,
  cpuMillis: TIER_BENCH_UNIT.cpuMillis * TIER_BENCH_LARGE_UNITS,
}

// Keep the edge's raw SSH listeners outside both outbound ephemeral ports and node pools.
export function nativeEdgePortRange(): readonly [number, number] {
  const end = Number(readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim().split(/\s+/)[1])
  const maximum = nodeSchema.components.schemas.PortSpec.properties.containerPort.maximum
  if (!Number.isInteger(end) || end >= maximum)
    throw new Error('Kernel leaves no external native edge port range')
  return [end + 1, maximum]
}
export const TEST_NODE_PORTS = 50
export const nativeNetworkName = (deployment: string) => `${deployment}-native`
// More than 1,500 concurrent-cohort creations plus scalar cohorts can rest without reuse.
export const TIER_BENCH_NODE_PORTS = 2048

export function nodePortLow(deployment: string, index: number) {
  const ephemeralStart = Number(
    readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim().split(/\s+/)[0],
  )
  const unprivilegedStart = Number(readFileSync('/proc/sys/net/ipv4/ip_unprivileged_port_start', 'utf8'))
  const slots = 90,
    stride = 2 * TEST_NODE_PORTS,
    base = ephemeralStart - (slots * stride + TIER_BENCH_NODE_PORTS),
    low = base + (Number.parseInt(deployment.slice(-3), 16) % slots) * stride + index * TEST_NODE_PORTS
  if (
    !Number.isInteger(low) ||
    !Number.isInteger(unprivilegedStart) ||
    base < unprivilegedStart ||
    low + TIER_BENCH_NODE_PORTS > ephemeralStart
  )
    throw new Error('Kernel port policy has no room for the test listener pool')
  return low
}

export const nodeConfigFor = (DEPLOYMENT: string) =>
  function nodeConfig(at: {
    home: string
    endpointUrl: string
    dir: string
    apiPort: number
    opsPort: number
    allowedImages?: string[]
    tier?: import('../src/isolation.ts').Isolation
    storeLease?: boolean
    nativeTools?: boolean
    agentPublicOrigin?: string
    index?: number
  }): string {
    const { home, endpointUrl, dir, apiPort, opsPort } = at
    const low = nodePortLow(DEPLOYMENT, at.index ?? 0)
    return `deployment_id = "${DEPLOYMENT}"
runtime = "${at.tier ?? 'container'}"
state_dir = "${join(home, 'state')}"
${at.nativeTools ? `[docker]\nnetwork = ${JSON.stringify(nativeNetworkName(DEPLOYMENT))}\nnative_tools_seccomp = ${JSON.stringify(process.env.PRYX_NATIVE_TOOLS_SECCOMP ?? new URL('../../deploy/native-tools-seccomp.json', import.meta.url).pathname)}\nnative_fuse = true\n${process.env.PRYX_NATIVE_TOOLS_APPARMOR ? `native_tools_apparmor = ${JSON.stringify(process.env.PRYX_NATIVE_TOOLS_APPARMOR)}\n` : ''}` : ''}
[api]
listen = "127.0.0.1:${apiPort}"
[ops]
listen = "127.0.0.1:${opsPort}"
[network]
port_range = [${low}, ${low + (process.env.PRYX_TIER_BENCH === '1' ? TIER_BENCH_NODE_PORTS : TEST_NODE_PORTS) - 1}]
${process.env.PRYX_TIER_BENCH === '1' ? '' : 'port_quarantine_seconds = 0'}
[wasm]
distribution = ${JSON.stringify(process.env.PRYX_AGENTOS_DISTRIBUTION ?? '/usr/local/lib/pryx/agentos')}
[workloads]
${at.agentPublicOrigin ? `agent_public_origin = ${JSON.stringify(at.agentPublicOrigin)}` : ''}
${process.env.PRYX_AGENT_BIN ? `agent_binary = ${JSON.stringify(process.env.PRYX_AGENT_BIN)}` : ''}
allowed_images = ${JSON.stringify(at.allowedImages ?? ['alpine:'])}
min_memory_mb = ${process.env.PRYX_TIER_BENCH === '1' ? TIER_BENCH_UNIT.memoryMb : 64}
[capacity]
${process.env.PRYX_TIER_BENCH === '1' ? '' : 'reserved_memory_mb = 0\nallocatable_memory_mb = 4096\n'}min_free_disk_mb = 256
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
