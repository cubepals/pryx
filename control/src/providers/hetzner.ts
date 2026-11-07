// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Hetzner Cloud servers on an existing private network/firewall; cloud-init carries the one-time join line. */

import { setTimeout as pause } from 'node:timers/promises'
import { StatusCodes } from 'http-status-codes'
import { MINUTE_MS, SECOND_MS } from '../durations.ts'
import { ProviderClient } from './client.ts'
import type { paths } from './generated/hetzner.ts'
import contract from './openapi/hetzner.openapi.json'
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

type Servers = paths['/servers']['get']['responses'][200]['content']['application/json']
type Created = paths['/servers']['post']['responses'][201]['content']['application/json']
type Create = paths['/servers']['post']['requestBody']['content']['application/json']
type Types = paths['/server_types']['get']['responses'][200]['content']['application/json']
export interface HetznerOptions {
  token: string
  deployment: string
  network: number
  firewall: number
  sshKeys: string[]
  image: string
  send?: typeof fetch
  joinAddress?: string
  userDataPrefix?: string
}
export class HetznerProvider implements Provider {
  readonly name = 'hetzner'
  readonly managed = true
  readonly client: ProviderClient
  constructor(readonly options: HetznerOptions) {
    this.client = new ProviderClient(contract.servers[0]?.url ?? '', options.token, options.send)
  }
  private host(server: Servers['servers'][number]): Host {
    return {
      id: String(server.id),
      region: server.location.name,
      address: server.private_net.find((n) => n.network === this.options.network)?.ip ?? null,
      requestKey: server.labels[ACQUIRE_LABEL] ?? '',
      shape: server.server_type.name,
    }
  }
  async list() {
    const hosts: Host[] = []
    let page: number | null = null
    do {
      const query: URLSearchParams = new URLSearchParams({
        label_selector: `${DEPLOYMENT_LABEL}=${this.options.deployment}`,
        ...(page ? { page: String(page) } : {}),
      })
      const response: Servers = await this.client.request<Servers>(`/servers?${query}`)
      hosts.push(...response.servers.map((s) => this.host(s)))
      page = response.meta.pagination.next_page
    } while (page)
    return hosts
  }
  async acquire(region: string, shape: Shape, key: string, bootstrap: Bootstrap) {
    const existing = (await this.list()).find((h) => h.requestKey === key)
    if (existing) return existing
    if (!this.options.network || !this.options.firewall || !this.options.sshKeys.length)
      throw new ProviderError('refused', 'Hetzner needs private network, firewall and SSH keys')
    const body: Create = {
      name: `pryx-${key.replaceAll('-', '')}`,
      location: region,
      server_type: shape.name,
      image: this.options.image,
      networks: [this.options.network],
      firewalls: [{ firewall: this.options.firewall }],
      ssh_keys: this.options.sshKeys,
      public_net: { enable_ipv4: true, enable_ipv6: true },
      labels: { [DEPLOYMENT_LABEL]: this.options.deployment, [ACQUIRE_LABEL]: key },
      user_data: `#!/bin/sh\n${this.options.userDataPrefix ?? ''}\nfor attempt in $(seq 1 30); do (\n${bootstrapScript({ ...bootstrap, line: bootstrap.line + (this.options.joinAddress ? ` --address ${shellQuote(this.options.joinAddress)}` : '') }, key)}\n) && exit 0; sleep 10; done; exit 1\n`,
    }
    const response = await this.client.request<Created>('/servers', 'POST', body)
    return this.host(response.server)
  }
  async bootstrap(_host: Host, _bootstrap: Bootstrap, _key: string) {} // cloud-init runs after create, including if its answer was lost.
  async release(host: Host, _key: string) {
    await this.client.request(`/servers/${encodeURIComponent(host.id)}`, 'DELETE')
    const deadline = performance.now() + 2 * MINUTE_MS
    while (performance.now() < deadline) {
      try {
        await this.client.request(`/servers/${encodeURIComponent(host.id)}`)
      } catch (error) {
        if (error instanceof ProviderError && error.status === StatusCodes.NOT_FOUND) return
        throw error
      }
      await pause(SECOND_MS)
    }
    throw new ProviderError('uncertain', 'Server deletion has not completed; safe to retry by host id')
  }

  async shapes(region: string) {
    const types: Types['server_types'] = []
    let page: number | null = null
    do {
      const response: Types = await this.client.request<Types>(`/server_types${page ? `?page=${page}` : ''}`)
      types.push(...response.server_types)
      page = response.meta.pagination.next_page
    } while (page)
    // Currency belongs to Hetzner's pricing response, never a repository price constant.
    const pricing =
      await this.client.request<paths['/pricing']['get']['responses'][200]['content']['application/json']>(
        '/pricing',
      )
    return types
      .filter(
        (t) =>
          t.architecture === 'x86' &&
          t.locations.some(
            (l) =>
              l.name === region &&
              l.available &&
              (!l.deprecation || Date.parse(l.deprecation.unavailable_after) > Date.now()),
          ),
      )
      .flatMap((t) => {
        const price = t.prices.find((p) => p.location === region)
        return price
          ? [
              {
                name: t.name,
                cpus: t.cores,
                memoryGb: t.memory,
                diskGb: t.disk,
                price: {
                  currency: pricing.pricing.currency,
                  hour: price.price_hourly.gross,
                  month: price.price_monthly.gross,
                  source: 'https://docs.hetzner.cloud/reference/cloud#tag/server-types',
                },
              },
            ]
          : []
      })
  }
}
