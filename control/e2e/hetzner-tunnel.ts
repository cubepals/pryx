// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The live Hetzner test's path to a sandbox's agent: an edge on the runner, wired to control the way
 * deployment.ts wires one, and a forward of the agent's host port added to the SSH tunnel's master
 * once the sandbox is placed, since the box's firewall lets only SSH in. Not for the tunnel's fixed
 * forwards or the test's assertions, which stay in hetzner.e2e.test.ts.
 */
import { spawnSync } from 'node:child_process'
import type { AddressInfo } from 'node:net'
import { startEdge } from '../../edge/src/agent.ts'
import { edgeConfig } from '../../edge/src/config.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { NO_DESTINATION } from '../src/edge-service.ts'
import type { startControl } from '../src/main.ts'
import { progress, redact, wait } from './hetzner-evidence.ts'

type Control = Awaited<ReturnType<typeof startControl>>
type Edge = Awaited<ReturnType<typeof startEdge>>
const LOOPBACK = ['127.0.0.1', 'localhost']

export const agentRouteId = (sandbox: string) => `${sandbox}:${AGENT_ROUTE_PORT}`

/** The ssh options that make the tunnel a master other ssh calls can add forwards to. */
export const tunnelMaster = (dir: string) => [
  '-o',
  'ControlMaster=yes',
  '-o',
  `ControlPath=${dir}/tunnel.sock`,
]

/** Starts an edge on ephemeral loopback ports and points control's agent credentials at it. */
export async function startRunnerEdge(control: Control, name: string): Promise<Edge> {
  const edge = await startEdge({
    ...edgeConfig({
      PRYX_CONTROL_URL: control.apiUrl,
      PRYX_EDGE_TOKEN: control.config.edgeToken,
      PRYX_EDGE_HOST: '127.0.0.1',
      PRYX_EDGE_ID: `${name}-runner`,
    }),
    httpPort: 0,
    tlsPort: 0,
    metricsPort: 0,
  })
  const port = (edge.listeners.get('http:80')?.address() as AddressInfo).port
  if (!control.edge.agents) {
    await edge.stop()
    throw new Error('Control has no agent capability service')
  }
  control.edge.agents.localOrigin = `http://127.0.0.1:${port}`
  progress('edge started', `http on 127.0.0.1:${port}`)
  return edge
}

/**
 * Waits for the sandbox's agent route, forwards its loopback port on the box to the same port on
 * the runner through the tunnel's master, and waits for the edge to hold the route, so the exec's
 * readiness probe starts with the whole path in place.
 */
export async function forwardAgent(
  control: Control,
  edge: Edge,
  sandbox: string,
  ssh: string[],
  dir: string,
  address: string,
) {
  const id = agentRouteId(sandbox)
  let seen = ''
  const route = await wait(
    `agent route of ${sandbox}`,
    async () => {
      const r = await control.edge.route(id)
      seen = r ? `destination ${r.destination}, agent ${r.agent ? 'present' : 'absent'}` : 'no route'
      return r?.agent && r.destination !== NO_DESTINATION ? r : null
    },
    2,
    () => seen,
  )
  const at = route.destination.lastIndexOf(':')
  const host = route.destination.slice(0, at),
    port = Number(route.destination.slice(at + 1))
  progress('agent destination', route.destination)
  if (!LOOPBACK.includes(host) || !Number.isInteger(port) || port <= 0)
    throw new Error(
      `The agent of ${sandbox} is at ${route.destination}; the live test's SSH tunnel can only serve a loopback agent address`,
    )
  const added = spawnSync(
    ssh[0] ?? 'ssh',
    [
      ...ssh.slice(1),
      '-S',
      `${dir}/tunnel.sock`,
      '-O',
      'forward',
      '-L',
      `127.0.0.1:${port}:127.0.0.1:${port}`,
      `root@${address}`,
    ],
    { encoding: 'utf8' },
  )
  if (added.status !== 0)
    throw new Error(
      `Forwarding the agent port ${port} through the tunnel failed (exit ${added.status ?? added.signal}): ${added.error?.message ?? added.stderr.trim()}`,
    )
  progress('agent forward added', `runner 127.0.0.1:${port} to the box's 127.0.0.1:${port}`)
  await wait(
    `edge holding ${id}`,
    async () => (edge.table.held.get(id)?.destination === route.destination ? true : null),
    2,
  )
}

/** Control's agent route for the sandbox: where the edge would dial it, and whether it has an agent manifest. */
export async function agentEvidence(control: Control, sandbox: string) {
  const route = await control.edge.route(agentRouteId(sandbox))
  console.error(
    'CONTROL agent route',
    redact(
      route
        ? JSON.stringify({
            id: route.id,
            destination: route.destination,
            manifest: Boolean(route.agent?.manifest),
          })
        : `none for ${sandbox}`,
    ),
  )
}
