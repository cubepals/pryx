// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Private authenticated HTTP transport and wire types. No endpoint convenience methods or mutation retries. */
import type { components } from './generated/api.ts'
export type Sandbox = components['schemas']['Sandbox']
export type MakeRequest = components['schemas']['MakeRequest']
export type ExecRequest = components['schemas']['ExecRequest']
export type ExecEvent = components['schemas']['ExecEvent']
export type Snapshot = components['schemas']['Snapshot']
export class PryxError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details?: unknown,
  ) {
    super(message)
    this.name = 'PryxError'
  }
}
export class Transport {
  readonly baseUrl: string
  readonly fetch: typeof fetch
  constructor(readonly options: { url: string; key: string; fetch?: typeof fetch }) {
    Object.defineProperty(this, 'options', { enumerable: false })
    this.baseUrl = options.url.replace(/\/$/, '')
    this.fetch = options.fetch ?? globalThis.fetch
  }
  async raw(path: string, method = 'GET', body?: unknown, key?: string, signal?: AbortSignal) {
    const response = await this.fetch(`${this.baseUrl}${path}`, {
      method,
      redirect: 'manual',
      headers: {
        authorization: `Bearer ${this.options.key}`,
        ...(body ? { 'content-type': 'application/json' } : {}),
        ...(key ? { 'Idempotency-Key': key } : {}),
      },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal,
    })
    return this.checked(response)
  }
  private async checked(response: Response) {
    if (!response.ok) {
      const data = (await response.json().catch(() => ({}))) as {
        error?: { code?: string; message?: string; details?: unknown }
      }
      throw new PryxError(
        response.status,
        data.error?.code ?? 'http_error',
        data.error?.message ?? response.statusText,
        data.error?.details,
      )
    }
    return response
  }
  async transfer(path: string, method = 'GET', body?: BodyInit) {
    return this.checked(
      await this.fetch(`${this.baseUrl}${path}`, {
        method,
        redirect: 'manual',
        body,
        headers: { authorization: `Bearer ${this.options.key}` },
      }),
    )
  }
  async request<T>(path: string, method = 'GET', body?: unknown, key?: string): Promise<T> {
    return (await (await this.raw(path, method, body, key)).json()) as T
  }
}
export async function* ndjson<T>(response: Response): AsyncGenerator<T> {
  if (!response.body) return
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let rest = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      rest += decoder.decode(value, { stream: !done })
      let at = rest.indexOf('\n')
      while (at >= 0) {
        const line = rest.slice(0, at)
        rest = rest.slice(at + 1)
        if (line.trim()) yield JSON.parse(line) as T
        at = rest.indexOf('\n')
      }
      if (done) break
    }
    if (rest.trim()) yield JSON.parse(rest) as T
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
