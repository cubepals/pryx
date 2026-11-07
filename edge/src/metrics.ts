// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Bounded-label Prometheus counters; routing never reads metrics to make decisions. */
export class Metrics {
  readonly counters = new Map<string, number>()
  routes = 0
  version = 0
  holds = 0
  count(name: string, label?: string, count = 1) {
    const key = `pryx_edge_${name}${label ? `{reason="${label.replaceAll(/[^a-zA-Z0-9_-]/g, '_')}"}` : ''}`
    this.counters.set(key, (this.counters.get(key) ?? 0) + count)
  }
  render() {
    return (
      [
        `pryx_edge_routes_held ${this.routes}`,
        `pryx_edge_version_held ${this.version}`,
        `pryx_edge_holds ${this.holds}`,
        ...[...this.counters].map(([key, value]) => `${key} ${value}`),
      ].join('\n') + '\n'
    )
  }
}
