// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Boat VMs use native idempotent creation and command/file APIs; systemd and private networking are checked before joining. */
import { setTimeout as pause } from 'node:timers/promises'
import { MINUTE_MS, SECOND_MS } from '../durations.ts'
import { ProviderClient } from './client.ts'
import type { components } from './generated/boat.ts'
import contract from './openapi/boat.openapi.json'
import { privateIp } from './private-network.ts'
import {
  ACQUIRE_LABEL,
  type Bootstrap,
  bootstrapScript,
  DEPLOYMENT_LABEL,
  type Host,
  type Provider,
  ProviderError,
  type Shape,
  shellQuote,
} from './provider.ts'

type Sandbox = components['schemas']['Sandbox']
type Command = components['schemas']['CommandResponse']
const DEFAULT_COMMAND_SECONDS = contract.components.schemas.CommandRequest.properties.timeoutSeconds.default
const MAX_COMMAND_SECONDS = contract.components.schemas.CommandRequest.properties.timeoutSeconds.maximum
export class BoatProvider implements Provider {
  readonly name = 'boat'
  readonly managed = true
  readonly client: ProviderClient
  constructor(
    readonly options: {
      token: string
      deployment: string
      region: string
      catalog: Shape[]
      networkSetup: string
      privateAddressCommand: string
      send?: typeof fetch
    },
  ) {
    this.client = new ProviderClient(contract.servers[0]?.url ?? '', options.token, options.send)
  }
  private host(s: Sandbox): Host {
    return {
      id: s.id,
      region: this.options.region,
      address: null,
      requestKey: s.labels?.[ACQUIRE_LABEL] ?? '',
      shape: s.type ?? '',
    }
  }
  async list() {
    const hosts: Host[] = []
    let cursor: string | undefined
    do {
      const query = new URLSearchParams({
        label: `${DEPLOYMENT_LABEL}=${this.options.deployment}`,
        ...(cursor ? { cursor } : {}),
      })
      const response = await this.client.request<{
        sandboxes: Sandbox[]
        pageInfo?: { hasMore?: boolean; nextCursor?: string | null }
      }>(`/sandboxes?${query}`)
      hosts.push(...response.sandboxes.filter((s) => s.state !== 'cancelled').map((s) => this.host(s)))
      cursor = response.pageInfo?.hasMore ? (response.pageInfo.nextCursor ?? undefined) : undefined
    } while (cursor)
    return hosts
  }
  private command(id: string, command: string, timeoutSeconds: number) {
    return this.client.request<Command>(`/sandboxes/${encodeURIComponent(id)}/commands`, 'POST', {
      command,
      timeoutSeconds,
    })
  }
  async acquire(region: string, shape: Shape, key: string, bootstrap: Bootstrap) {
    if (region !== this.options.region)
      throw new ProviderError('refused', 'Boat has no requested geographic region selector')
    if (!this.options.networkSetup || !this.options.privateAddressCommand)
      throw new ProviderError('refused', 'Boat needs private-network configuration before renting')
    let host = (await this.list()).find((h) => h.requestKey === key)
    if (!host) {
      const body: components['schemas']['CreateSandboxRequest'] = {
        type: shape.name as components['schemas']['CreateSandboxRequest']['type'],
        name: `pryx-${key}`,
        noEnv: true,
        ttlSeconds: null,
        labels: { [DEPLOYMENT_LABEL]: this.options.deployment, [ACQUIRE_LABEL]: key },
      }
      const response = await this.client.request<{ sandbox: Sandbox }>('/sandboxes', 'POST', body, {
        'Idempotency-Key': key,
      })
      host = this.host(response.sandbox)
    }
    await this.bootstrap(host, bootstrap, key)
    return host
  }
  async bootstrap(host: Host, bootstrap: Bootstrap, key: string) {
    if (!this.options.networkSetup || !this.options.privateAddressCommand)
      throw new ProviderError(
        'refused',
        'Boat needs an explicit private-network bootstrap and address command',
      )
    const deadline = performance.now() + 10 * MINUTE_MS
    let probe: Command | undefined
    while (performance.now() < deadline) {
      try {
        probe = await this.command(
          host.id,
          'sudo docker info >/dev/null && test -d /run/systemd/system && sudo systemctl show-environment >/dev/null',
          DEFAULT_COMMAND_SECONDS,
        )
        if (probe.exitCode === 0) break
      } catch {}
      await pause(SECOND_MS)
    }
    if (probe?.exitCode !== 0)
      throw new ProviderError(
        'uncertain',
        'Boat VM did not prove Docker and systemd support; host retained for inspection',
      )
    const path = `/home/user/.pryx/bootstrap-${key}.sh`
    const runFile = async (script: string) => {
      const prepared = await this.command(
        host.id,
        `install -d -m 0700 /home/user/.pryx; touch ${shellQuote(path)}; chmod 0600 ${shellQuote(path)}`,
        DEFAULT_COMMAND_SECONDS,
      )
      if (prepared.exitCode !== 0)
        throw new ProviderError('uncertain', 'Boat private bootstrap directory could not be prepared')
      const body: components['schemas']['FileWriteRequest'] = {
        path,
        content: `set -eu\n${script}\n`,
        encoding: 'utf8',
      }
      await this.client.request(`/sandboxes/${encodeURIComponent(host.id)}/files`, 'PUT', body)
      const result = await this.command(
        host.id,
        `sudo install -d -m 0700 /var/lib/pryx-provider; sudo flock -x /var/lib/pryx-provider/bootstrap.lock sh ${shellQuote(path)}; result=$?; rm -f ${shellQuote(path)}; exit "$result"`,
        MAX_COMMAND_SECONDS,
      )
      if (result.exitCode !== 0)
        throw new ProviderError('uncertain', 'Boat bootstrap command did not complete')
    }
    await runFile(this.options.networkSetup)
    const address = await this.command(host.id, this.options.privateAddressCommand, DEFAULT_COMMAND_SECONDS),
      ip = address.stdout.trim()
    if (address.exitCode !== 0 || !privateIp(ip))
      throw new ProviderError('uncertain', 'Boat did not report a configured private IP')
    host.address = ip
    await runFile(
      bootstrapScript({ ...bootstrap, line: `${bootstrap.line} --address ${shellQuote(ip)}` }, key),
    )
  }

  async release(host: Host, _key: string) {
    await this.client.request(`/sandboxes/${encodeURIComponent(host.id)}`, 'DELETE', undefined, {
      'X-Ascii-Confirm-Delete': host.id,
    })
  }
  async shapes(region: string) {
    return region === this.options.region ? this.options.catalog : []
  }
}
