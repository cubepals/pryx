// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * How much the edge lets in, counted per server and per address, so a flood on one server is
 * refused there and every other server keeps taking connections. Pure counting: it never touches a
 * socket, and decides nothing about what a refused client is told (that is `front.ts`'s).
 *
 * - An address (an IPv6 one by its /64, which one machine usually holds whole) may hold only a
 *   few connections that haven't said where they are going, only so many that have, and open only
 *   so many a second, pings and joins alike.
 * - A server may take only so many new connections a second, and hold only so many open.
 */

import { SECOND_MS } from '../../control/src/durations.ts'

/** A token bucket's shape: `burst` at once, then `perSecond` more each second. */
export interface Rate {
  perSecond: number
  burst: number
}

export interface Limits {
  /** `open` counts the connections let through that are still open, across every server. */
  address: { connections: Rate; greeting: number; open: number }
  server: { connections: Rate; open: number }
}

/** Which limit refused a connection. */
export type Refusal = 'address-rate' | 'address-greeting' | 'address-open' | 'server-rate' | 'server-open'

export class Bucket {
  readonly rate: Rate
  #tokens: number
  #at: number

  constructor(rate: Rate, now: number) {
    this.rate = rate
    this.#tokens = rate.burst
    this.#at = now
  }

  #fill(now: number): void {
    this.#tokens = Math.min(
      this.rate.burst,
      this.#tokens + ((now - this.#at) / SECOND_MS) * this.rate.perSecond,
    )
    this.#at = now
  }

  take(now: number): boolean {
    this.#fill(now)
    if (this.#tokens < 1) return false
    this.#tokens -= 1
    return true
  }

  /** Whether it has filled back up, so forgetting it changes nothing. */
  full(now: number): boolean {
    this.#fill(now)
    return this.#tokens >= this.rate.burst
  }
}

/** What is counted against one address or one server. */
interface Tally {
  bucket: Bucket
  /** Connections still open: greeting ones for an address, admitted ones for a server. */
  open: number
}

/** The key an address is counted under: an IPv4 address as it is, an IPv6 one by its /64. */
export function addressKey(address: string): string {
  const mapped = address.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/i)
  if (mapped?.[1]) return mapped[1]
  if (!address.includes(':')) return address
  const [head = '', tail = ''] = address.toLowerCase().split('::')
  const left = head === '' ? [] : head.split(':')
  const right = tail === '' ? [] : tail.split(':')
  const groups = address.includes('::')
    ? [...left, ...Array<string>(8 - left.length - right.length).fill('0'), ...right]
    : left
  return `${groups
    .slice(0, 4)
    .map((group) => group.replace(/^0+(?=.)/, ''))
    .join(':')}::/64`
}

export class Gate {
  readonly #addresses = new Map<string, Tally>()
  readonly #servers = new Map<string, Tally>()
  /** Connections let through and still open, by address key; an address is gone once it has none. */
  readonly #admitted = new Map<string, number>()
  /** Refusals since the last `sweep`, by limit and by what it counted. */
  readonly #refused = new Map<string, number>()

  readonly limits: Limits
  readonly now: () => number

  constructor(limits: Limits, now: () => number = Date.now) {
    this.limits = limits
    this.now = now
  }

  /**
   * A connection from `address` opened. Null refuses it; otherwise call what comes back once it
   * has said where it is going, or closed first.
   */
  greet(address: string): (() => void) | null {
    const key = addressKey(address)
    const tally = this.#tally(this.#addresses, key, this.limits.address.connections)
    if (tally.open >= this.limits.address.greeting) return this.#refuse('address-greeting', key)
    if (!tally.bucket.take(this.now())) return this.#refuse('address-rate', key)
    tally.open++
    return once(() => tally.open--)
  }

  /**
   * A connection from `address` said it is going to `server`. Null refuses it; otherwise call what
   * comes back once it closes. A null `address` is counted against the server alone.
   */
  admit(server: string, address: string | null = null): (() => void) | null {
    const key = address === null ? null : addressKey(address)
    const admitted = key === null ? 0 : (this.#admitted.get(key) ?? 0)
    if (key !== null && admitted >= this.limits.address.open) return this.#refuse('address-open', key)
    const tally = this.#tally(this.#servers, server, this.limits.server.connections)
    if (tally.open >= this.limits.server.open) return this.#refuse('server-open', server)
    if (!tally.bucket.take(this.now())) return this.#refuse('server-rate', server)
    tally.open++
    if (key !== null) this.#admitted.set(key, admitted + 1)
    return once(() => {
      tally.open--
      if (key !== null) this.#release(key)
    })
  }

  #release(key: string): void {
    const left = (this.#admitted.get(key) ?? 1) - 1
    if (left > 0) this.#admitted.set(key, left)
    else this.#admitted.delete(key)
  }

  /** Forgets what no longer limits anything, and hands back the refusals since the last sweep. */
  sweep(): Map<string, number> {
    const now = this.now()
    for (const tallies of [this.#addresses, this.#servers])
      for (const [key, tally] of tallies) if (tally.open === 0 && tally.bucket.full(now)) tallies.delete(key)
    const refused = new Map(this.#refused)
    this.#refused.clear()
    return refused
  }

  #tally(tallies: Map<string, Tally>, key: string, rate: Rate): Tally {
    let tally = tallies.get(key)
    if (tally === undefined) {
      tally = { bucket: new Bucket(rate, this.now()), open: 0 }
      tallies.set(key, tally)
    }
    return tally
  }

  #refuse(why: Refusal, key: string): null {
    const counted = `${why} ${key}`
    this.#refused.set(counted, (this.#refused.get(counted) ?? 0) + 1)
    return null
  }
}

/** `release`, made safe to call more than once: a socket can both end and fail. */
function once(release: () => void): () => void {
  let done = false
  return () => {
    if (done) return
    done = true
    release()
  }
}
