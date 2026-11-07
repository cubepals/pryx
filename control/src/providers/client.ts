// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** One bounded provider HTTP attempt. Durable idempotency and uncertain outcomes are handled by the acquisition ledger. */
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../durations.ts'
import { ProviderError } from './provider.ts'
export class ProviderClient {
  constructor(
    readonly base: string,
    private readonly token: string,
    readonly send: typeof fetch = fetch,
    readonly requestMs = 30 * SECOND_MS,
  ) {}
  async request<T>(
    path: string,
    method = 'GET',
    body?: unknown,
    headers: Record<string, string> = {},
  ): Promise<T> {
    let response: Response
    try {
      response = await this.send(`${this.base.replace(/\/$/, '')}${path}`, {
        method,
        headers: { authorization: `Bearer ${this.token}`, 'content-type': 'application/json', ...headers },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal: AbortSignal.timeout(this.requestMs),
      })
    } catch {
      throw new ProviderError('uncertain', 'Provider did not answer')
    }
    if (response.status === StatusCodes.NOT_FOUND && method === 'DELETE') return null as T
    if (!response.ok)
      throw new ProviderError(
        response.status >= StatusCodes.INTERNAL_SERVER_ERROR ||
          response.status === StatusCodes.REQUEST_TIMEOUT
          ? 'uncertain'
          : 'refused',
        `Provider refused with HTTP ${response.status}`,
        response.status,
      )
    if (response.status === StatusCodes.NO_CONTENT) return null as T
    return (await response.json().catch(() => null)) as T
  }
}
