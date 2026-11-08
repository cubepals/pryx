// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Incremental routes and connection ownership. Epoch high-water marks survive removal and reject delayed lists. */
import type { EdgeRoute, EdgeRoutes } from './wire.ts'
export class RouteTable {
  readonly held = new Map<string, EdgeRoute>()
  readonly epochs = new Map<string, number>()
  readonly connections = new Set<{
    route: string
    sandbox: string
    epoch: number
    destination: string
    close: () => void
  }>()
  constructor(readonly stale: () => void = () => {}) {}
  apply(list: EdgeRoutes) {
    const wanted = new Set(list.routes.map((r) => r.id))
    for (const route of list.routes) {
      if (route.epoch < (this.epochs.get(route.sandbox) ?? 0)) continue
      this.epochs.set(route.sandbox, route.epoch)
      const previous = this.held.get(route.id)
      this.held.set(route.id, route)
      for (const c of [...this.connections])
        if (
          c.sandbox === route.sandbox &&
          (c.epoch < route.epoch ||
            (c.route === route.id &&
              (c.destination !== route.destination ||
                route.state !== undefined ||
                previous?.agent?.manifest.publicKey !== route.agent?.manifest.publicKey ||
                previous?.ssh?.session !== route.ssh?.session ||
                previous?.ssh?.targetPort !== route.ssh?.targetPort)))
        ) {
          this.stale()
          c.close()
          this.connections.delete(c)
        }
      if (previous && previous.sandbox !== route.sandbox) throw new Error('Route identity changed owner')
    }
    for (const [id] of this.held)
      if (!wanted.has(id)) {
        this.held.delete(id)
        for (const c of [...this.connections])
          if (c.route === id) {
            this.stale()
            c.close()
            this.connections.delete(c)
          }
      }
  }
  find(name: string, front: EdgeRoute['front'], port: number) {
    return [...this.held.values()].find(
      (r) =>
        !r.agent &&
        r.epoch >= (this.epochs.get(r.sandbox) ?? 0) &&
        r.front === front &&
        r.publicPort === port &&
        (front === 'port' || r.hostname === name),
    )
  }
  track(route: EdgeRoute, destination: string, epoch: number, close: () => void) {
    const held = this.held.get(route.id)
    if (!held || epoch < (this.epochs.get(route.sandbox) ?? 0) || held.epoch > epoch) {
      close()
      return () => {}
    }
    const c = { route: route.id, sandbox: route.sandbox, destination, epoch, close }
    this.connections.add(c)
    return () => this.connections.delete(c)
  }
}
