// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Asking the control plane for routes (`edge-poll.ts`): the list comes only when it changed, with
 * its version. The version said back is the edge's word that it holds every route of that list, so
 * it is kept only once the list is applied, and given at once by asking again.
 *
 * Not for applying a list, which the caller does (`router.ts`).
 */
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import {
  EDGE_STARTED_HEADER,
  ROUTES_HELD_HEADER,
  ROUTES_VERSION_HEADER,
} from '../../control/src/edge-headers.ts'
import type { EdgeRoutes } from './wire.ts'

/** When this agent started: the control plane gives a restarted edge time to catch up. */
const STARTED_AT = Date.now()

export class RoutesPoll {
  /** Asks for the routes with these headers. */
  readonly #ask: (headers: Record<string, string>) => Promise<Response>
  /** Applies a list the control plane sent; whether every route of it is held. */
  readonly #take: (list: EdgeRoutes) => Promise<boolean>
  /** The ETag of the list every route of which is held, if one is: it spares sending it again. */
  #etag: string | null = null
  /** The version of the newest list every route of which was held. */
  #version: string | null = null
  /** When the control plane last answered that the held list is its newest, or sent one now held. */
  #currentAt: number | null = null

  constructor(
    ask: (headers: Record<string, string>) => Promise<Response>,
    take: (list: EdgeRoutes) => Promise<boolean>,
  ) {
    this.#ask = ask
    this.#take = take
  }

  /** Asks once, and applies the list if it changed; whether it did and is held now. */
  async sync(): Promise<boolean> {
    const response = await this.#ask({
      [EDGE_STARTED_HEADER]: String(STARTED_AT),
      ...(this.#etag === null ? {} : { 'if-none-match': this.#etag }),
      ...(this.#version === null ? {} : { [ROUTES_HELD_HEADER]: this.#version }),
    })
    if (response.status === StatusCodes.NOT_MODIFIED) {
      if (this.#version !== null) this.#currentAt = Date.now()
      return false
    }
    if (!response.ok) throw new Error(`routes answered ${response.status}`)
    if (!(await this.#take((await response.json()) as EdgeRoutes))) return false
    this.#etag = response.headers.get('etag')
    this.#version = response.headers.get(ROUTES_VERSION_HEADER)
    this.#currentAt = Date.now()
    this.#first.resolve()
    return true
  }

  readonly #first = Promise.withResolvers<void>()

  /** Resolves once a list has been held whole, the first since this agent started. */
  get firstHold(): Promise<void> {
    return this.#first.promise
  }

  /** Whether a list has been held whole since this agent started. */
  get version(): number {
    return Number(this.#version ?? 0)
  }

  heldOnce(): boolean {
    return this.#version !== null
  }

  /** Whether the list held was the control plane's newest when it was last asked, within `withinMs`. */
  current(withinMs = 5 * SECOND_MS): boolean {
    return this.#currentAt !== null && Date.now() - this.#currentAt <= withinMs
  }

  /**
   * Something it held was lost, as a route mc-router refused: the next ask fetches the list again.
   * The version it said stays: a newer one is said only once a newer list is held whole.
   */
  forget(): void {
    this.#etag = null
  }
}
