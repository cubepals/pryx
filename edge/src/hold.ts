// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Coalesce wake requests per sandbox and bound silence by a monotonic deadline. No bytes are sent to the client. */
import { setTimeout as pause } from 'node:timers/promises'
import { SECOND_MS } from '../../control/src/durations.ts'
import type { EdgeRoute, WakeResult } from './wire.ts'
export class Hold {
  readonly pending = new Map<string, Promise<WakeResult>>()
  constructor(
    readonly ask: (route: string, signal: AbortSignal) => Promise<WakeResult>,
    readonly requestMs = 3 * SECOND_MS,
  ) {}
  async wake(route: EdgeRoute, signal: AbortSignal) {
    let pending = this.pending.get(route.sandbox)
    if (!pending) {
      pending = this.ask(route.id, AbortSignal.timeout(this.requestMs)).finally(() =>
        this.pending.delete(route.sandbox),
      )
      this.pending.set(route.sandbox, pending)
    }
    signal.throwIfAborted()
    return new Promise<WakeResult>((resolve, reject) => {
      const abort = () => reject(signal.reason)
      signal.addEventListener('abort', abort, { once: true })
      pending.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort))
    })
  }
  async ready(route: EdgeRoute, signal: AbortSignal): Promise<{ destination: string; epoch: number }> {
    while (!signal.aborted) {
      const result = await this.wake(route, signal)
      if (result.outcome === 'ready') return result
      if (result.outcome === 'denied') throw new Error('Wake denied')
      await pause(SECOND_MS / 4, undefined, { signal })
    }
    throw new Error('Hold elapsed')
  }
}
