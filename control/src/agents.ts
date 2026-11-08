// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Lifecycle-bound capabilities and stopped sandbox data. Running agent requests never call this service. */
import { setTimeout as pause } from 'node:timers/promises'
import { z } from 'zod'
import agentContract from '../../agent/openapi.json'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { SECOND_MS } from './durations.ts'
import { routesPending } from './edge-routed.ts'
import { decodeHandle, encodeHandle, type FleetRef } from './handle.ts'
import type { NodeClient } from './node-client.ts'
import { projectRow } from './project-admission.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import type { AgentTokenResponse } from './wire.ts'

export const sandboxTokenSchema = z.object({
  token: z.string(),
  expiresAt: z.number().int(),
  epoch: z.number().int(),
  url: z.string(),
  publicKey: z.string(),
})
export class Agents {
  localOrigin?: string
  initializeMounts?: (
    id: string,
    project: string,
    credential: z.infer<typeof sandboxTokenSchema>,
  ) => Promise<void>
  capabilityVersion() {
    const policy = nodeSchema['x-pryx-agent']
    return Math.floor(Date.now() / ((policy.tokenLifetimeSeconds - policy.tokenClockSkewSeconds) * SECOND_MS))
  }
  readonly routeTokens = new Map<string, AgentTokenResponse>()
  constructor(
    readonly boxes: Sandboxes,
    readonly nodes: NodeClient,
    readonly domain: string,
  ) {}
  async data(
    id: string,
    project: string,
    operation: 'content' | 'list' | 'stat',
    method: string,
    path: string,
    signal: AbortSignal,
    body?: ReadableStream<Uint8Array>,
  ) {
    const row = await this.boxes.record(id, project)
    if (!row.handle) throw new SandboxError('not_running', 'Sandbox data has no local node')
    const node = await this.boxes.engine.home(row.handle)
    if (!node) throw new SandboxError('not_running', 'Sandbox data has no local node')
    return this.nodes.transfer(
      node,
      method,
      `/v1/workloads/${id}/files/${operation}?${new URLSearchParams({ path })}`,
      { epoch: decodeHandle(row.handle).epoch, signal, body },
    )
  }

  async token(id: string, project: string, requestUrl: string) {
    const row = await this.boxes.record(id, project)
    if (!row.handle) throw new SandboxError('not_running', 'Wake the sandbox before requesting its agent')
    const ref = decodeHandle(row.handle)
    if (!ref.agent) throw new SandboxError('unsupported', 'This sandbox has no agent')
    // Lifecycle may rotate a signing key before an edge's next poll. Delay only token issuance
    // until the current route is acknowledged; ready agent operations never enter this path.
    while (await routesPending(this.boxes.db, id)) await pause(SECOND_MS / 20)
    const node = await this.boxes.engine.home(row.handle)
    if (!node) throw new SandboxError('not_running', 'Sandbox has no local node')
    const policy = await projectRow(this.boxes.db, project, this.boxes.projects?.defaults)
    const expiry = [row.expires_at, policy.limits.expiresAt]
      .filter((v): v is string => typeof v === 'string')
      .map((v) => Math.floor(Date.parse(v) / SECOND_MS))
    const answer = await this.nodes.call<AgentTokenResponse>(
      node,
      'POST',
      `/v1/workloads/${id}/agent-token`,
      {
        epoch: ref.epoch,
        body: {
          scope: 'sandbox',
          maxPorts: row.spec.sparePorts ?? 0,
          maxExecSeconds: policy.limits.maxExecTimeout ?? null,
          expiresAt: expiry.length ? Math.min(...expiry) : null,
        },
      },
    )
    const host = new URL(requestUrl).hostname
    const local = ['127.0.0.1', 'localhost', '[::1]'].includes(host)
    const credential = {
      token: answer.token,
      expiresAt: answer.expiresAt,
      epoch: answer.epoch,
      publicKey: answer.agent.publicKey,
      url: this.localOrigin
        ? `${this.localOrigin}/sandboxes/${id}`
        : local
          ? `http://${host}/sandboxes/${id}`
          : `https://${id}.${this.domain}`,
    }
    let mountsPending = false
    if (row.state === 'running') {
      const deadline = Date.now() + 5 * SECOND_MS
      let ready = false
      let last = 'no response'
      while (Date.now() < deadline) {
        try {
          const response = await fetch(`${credential.url}/_pryx/health`, {
            headers: { authorization: `Bearer ${credential.token}` },
            signal: AbortSignal.timeout(Math.max(1, deadline - Date.now())),
            redirect: 'manual',
          })
          last = String(response.status)
          if (response.ok && row.spec.fuseMounts) {
            const health = (await response.json()) as { toolsVersion?: number; mountsPending?: boolean }
            if ((health.toolsVersion ?? 0) < agentContract['x-pryx-tools'].version)
              throw new SandboxError('unsupported', 'This agent cannot initialize native mounts')
            mountsPending = health.mountsPending === true
          } else await response.body?.cancel()
          if (response.ok) {
            ready = true
            break
          }
        } catch (error) {
          if (error instanceof SandboxError && error.code === 'unsupported') throw error
          last = error instanceof Error ? error.name : 'transport failure'
        }
        await pause(SECOND_MS / 100)
      }
      if (!ready) {
        console.error('Agent token readiness failed', {
          sandbox: id,
          state: row.state,
          epoch: ref.epoch,
          last,
        })
        throw new SandboxError('not_running', `Agent route did not become ready (${last})`)
      }
    }
    if (mountsPending) await this.initializeMounts?.(id, project, credential)
    return credential
  }

  async routeToken(ref: FleetRef): Promise<string | undefined> {
    if (!ref.node || !ref.agent) return undefined
    const key = `${ref.key}:${ref.epoch}:${ref.agent.publicKey}:${this.capabilityVersion()}`
    const cached = this.routeTokens.get(key)
    if (cached && cached.expiresAt * SECOND_MS > Date.now() + 30 * SECOND_MS) return cached.token
    const node = await this.boxes.engine.home(encodeHandle(ref))
    if (!node) return undefined
    const issued = await this.nodes.call<AgentTokenResponse>(
      node,
      'POST',
      `/v1/workloads/${ref.key}/agent-token`,
      {
        epoch: ref.epoch,
        body: { scope: 'routes', maxPorts: 0, maxExecSeconds: null, expiresAt: null },
      },
    )
    for (const previous of this.routeTokens.keys())
      if (previous.startsWith(`${ref.key}:`)) this.routeTokens.delete(previous)
    this.routeTokens.set(key, issued)
    return issued.token
  }
}
