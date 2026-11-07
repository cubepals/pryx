// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Deterministic host provider for scaling and lost-response tests; never contacts an external service. */
import type { Bootstrap, Host, Provider, Shape } from './provider.ts'
export class FakeProvider implements Provider {
  readonly name = 'fake'
  readonly managed = true
  readonly hosts = new Map<string, Host>()
  readonly released = new Set<string>()
  creates = 0
  loseNextReply = false
  loseNextReleaseReply = false
  constructor(readonly catalog: Shape[]) {}
  async acquire(region: string, shape: Shape, key: string, _bootstrap: Bootstrap) {
    let host = this.hosts.get(key)
    if (!host) {
      this.creates++
      host = {
        id: `fake-${key}`,
        region,
        address: `10.0.0.${this.creates}`,
        requestKey: key,
        shape: shape.name,
      }
      this.hosts.set(key, host)
    }
    if (this.loseNextReply) {
      this.loseNextReply = false
      throw new Error('Response lost after creation')
    }
    return host
  }
  async bootstrap(_host: Host, _bootstrap: Bootstrap, _key: string) {}
  async release(host: Host, _key: string) {
    this.released.add(host.id)
    if (this.loseNextReleaseReply) {
      this.loseNextReleaseReply = false
      throw new Error('Release reply lost')
    }
  }
  async list() {
    return [...this.hosts.values()].filter((h) => !this.released.has(h.id))
  }
  async shapes(_region: string) {
    return this.catalog
  }
}
