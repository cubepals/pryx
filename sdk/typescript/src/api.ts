// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The public pryx client, typed by the committed API document. Mutations are never retried automatically. */
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
export class Pryx {
  readonly baseUrl: string
  readonly fetch: typeof fetch
  constructor(readonly options: { baseUrl: string; apiKey: string; fetch?: typeof fetch }) {
    Object.defineProperty(this, 'options', { enumerable: false })
    this.baseUrl = options.baseUrl.replace(/\/$/, '')
    this.fetch = options.fetch ?? globalThis.fetch
  }
  async raw(path: string, method = 'GET', body?: unknown, key?: string, signal?: AbortSignal) {
    const response = await this.fetch(`${this.baseUrl}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${this.options.apiKey}`,
        ...(body ? { 'content-type': 'application/json' } : {}),
        ...(key ? { 'Idempotency-Key': key } : {}),
      },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal,
    })
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
  async request<T>(path: string, method = 'GET', body?: unknown, key?: string): Promise<T> {
    return (await (await this.raw(path, method, body, key)).json()) as T
  }
  make(body: MakeRequest, key: string = crypto.randomUUID()) {
    return this.request<Sandbox>('/v1/sandboxes', 'POST', body, key)
  }
  list() {
    return this.request<{ sandboxes: Sandbox[] }>('/v1/sandboxes')
  }
  project() {
    return this.request<components['schemas']['Project']>('/v1/project')
  }
  get(id: string) {
    return this.request<Sandbox>(`/v1/sandboxes/${encodeURIComponent(id)}`)
  }
  delete(id: string) {
    return this.request<{ existed: boolean }>(`/v1/sandboxes/${encodeURIComponent(id)}`, 'DELETE')
  }
  action(id: string, verb: 'start' | 'stop' | 'sleep' | 'wake' | 'rest') {
    return this.request<{ sandbox: Sandbox; tier?: string }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/${verb}`,
      'POST',
    )
  }
  async *exec(id: string, body: ExecRequest, key: string = crypto.randomUUID()): AsyncGenerator<ExecEvent> {
    yield* ndjson<ExecEvent>(
      await this.raw(`/v1/sandboxes/${encodeURIComponent(id)}/exec`, 'POST', body, key),
    )
  }
  async *logs(
    id: string,
    query: { tail?: number; since?: string; follow?: boolean } = {},
    signal?: AbortSignal,
  ): AsyncGenerator<unknown> {
    const params = new URLSearchParams(Object.entries(query).map(([k, v]) => [k, String(v)]))
    yield* ndjson(
      await this.raw(
        `/v1/sandboxes/${encodeURIComponent(id)}/logs?${params}`,
        'GET',
        undefined,
        undefined,
        signal,
      ),
    )
  }
  snapshot(id: string) {
    return this.request<Snapshot>(`/v1/sandboxes/${encodeURIComponent(id)}/snapshots`, 'POST')
  }
  snapshots(id: string) {
    return this.request<{ snapshots: Snapshot[] }>(`/v1/sandboxes/${encodeURIComponent(id)}/snapshots`)
  }
  deleteSnapshot(id: string, snapshot: string) {
    return this.request<{ existed: boolean }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/snapshots/${encodeURIComponent(snapshot)}`,
      'DELETE',
    )
  }
  fork(
    snapshot: string,
    body: Omit<MakeRequest, 'image' | 'snapshot' | 'template'> = {},
    key: string = crypto.randomUUID(),
  ) {
    return this.request<Sandbox>(`/v1/snapshots/${encodeURIComponent(snapshot)}/sandboxes`, 'POST', body, key)
  }
  endpoints(id: string) {
    return this.request<{ endpoints: Sandbox['endpoints'] }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/endpoints`,
    )
  }
  events(id: string, since?: string) {
    return this.request<{ events: unknown[] }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/events${since ? `?since=${encodeURIComponent(since)}` : ''}`,
    )
  }
  putFile(id: string, path: string, dataBase64: string) {
    return this.request<{ written: boolean }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/files/${path.split('/').map(encodeURIComponent).join('/')}`,
      'PUT',
      { dataBase64 },
    )
  }
  getFile(id: string, path: string) {
    return this.request<{ archiveBase64: string }>(
      `/v1/sandboxes/${encodeURIComponent(id)}/files/${path.split('/').map(encodeURIComponent).join('/')}`,
    )
  }
  template(body: components['schemas']['TemplateRequest']) {
    return this.request('/v1/templates', 'POST', body)
  }
  templates() {
    return this.request('/v1/templates')
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
