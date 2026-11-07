// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The control plane's client for pryxd's API (pryxd's docs/protocol.md): mutual TLS with
 * the control plane's own client certificate, the fleet CA as the only root, and the node's name,
 * which the control plane chose at enrollment, as what the node's certificate must say. An address
 * is only where to dial; it never identifies a node.
 *
 * A call that is safe to repeat is sent again when the node doesn't answer, within the call's own
 * timeout; any other is sent once.
 */

import https from 'node:https'
import { connect as tcpConnect } from 'node:net'
import { Readable } from 'node:stream'
import { setTimeout as sleep } from 'node:timers/promises'
import { StatusCodes } from 'http-status-codes'
import { nodeName } from './ca.ts'
import { MINUTE_MS, SECOND_MS } from './durations.ts'

const EPOCH_HEADER = 'blocklyd-epoch'
export const CONFIRM_DELETE_HEADER = 'x-blockly-confirm-delete-data'

/** Where to dial a node: its id and pryxd's API address. */
export interface NodeAddress {
  id: string
  apiAddress: string
}

/** The node answered, and refused: `code` is pryxd's stable error code. */
export class NodeRefused extends Error {
  readonly status: number
  readonly code: string
  readonly details: unknown

  constructor(status: number, code: string, message: string, details: unknown) {
    super(message)
    this.name = 'NodeRefused'
    this.status = status
    this.code = code
    this.details = details
  }
}

/** The node didn't answer: nothing is known about whether the request took effect. */
export class NodeUnreachable extends Error {
  readonly kind: 'refused' | 'timeout' | 'unreachable' | 'reset' | 'tls' | 'other'

  constructor(kind: NodeUnreachable['kind'], message: string) {
    super(message)
    this.name = 'NodeUnreachable'
    this.kind = kind
  }
}

export interface CallOptions {
  body?: unknown
  /** The placement epoch the request acts for. */
  epoch?: number | null
  headers?: Record<string, string>
  timeoutMs?: number
}

export interface ClientIdentity {
  caPem: string
  certPem: string
  keyPem: string
}

export function splitAddress(address: string): { host: string; port: number } {
  const at = address.lastIndexOf(':')
  const host = address.slice(0, at).replace(/^\[|\]$/g, '')
  return { host, port: Number(address.slice(at + 1)) }
}

/**
 * The waits between attempts at a call that is safe to repeat, after no answer: a keep-alive
 * connection the node had closed, or pryxd restarting. The call's timeout bounds them all.
 */
const RETRY_DELAYS_MS = [0.25 * SECOND_MS, SECOND_MS, 2.5 * SECOND_MS]

/**
 * Whether a call may be sent again when nothing is known of the first: a read, a PUT of a spec,
 * and a start or stop, which pryxd answers the same however often it is asked. Not a kill, a
 * delete, a fence, an exec or a transfer.
 */
export function repeatable(method: string, path: string): boolean {
  if (method === 'GET' || method === 'PUT') return true
  return method === 'POST' && /^\/v1\/workloads\/[^/?]+\/(start|stop)$/.test(path)
}

function unreachable(error: NodeJS.ErrnoException): NodeUnreachable {
  const code = error.code ?? ''
  if (code === 'ECONNREFUSED') return new NodeUnreachable('refused', `connection refused: ${error.message}`)
  if (code === 'ETIMEDOUT' || code === 'TIMEOUT') return new NodeUnreachable('timeout', error.message)
  if (code === 'EHOSTUNREACH' || code === 'ENETUNREACH')
    return new NodeUnreachable('unreachable', error.message)
  if (code === 'ECONNRESET' || code === 'EPIPE') return new NodeUnreachable('reset', error.message)
  if (code.startsWith('ERR_TLS') || code.startsWith('ERR_SSL') || /CERT|SELF_SIGNED/.test(code))
    return new NodeUnreachable('tls', `${code}: ${error.message}`)
  return new NodeUnreachable('other', `${code || 'error'}: ${error.message}`)
}

export class NodeClient {
  #agent: https.Agent
  readonly #deployment: string

  constructor(identity: ClientIdentity, deployment: string) {
    this.#deployment = deployment
    this.#agent = NodeClient.#agentFor(identity)
  }

  static #agentFor(identity: ClientIdentity): https.Agent {
    return new https.Agent({
      ca: identity.caPem,
      cert: identity.certPem,
      key: identity.keyPem,
      keepAlive: true,
      maxSockets: 16,
      minVersion: 'TLSv1.3',
    })
  }

  /** A renewed client certificate: new connections present it; open ones finish on the old. */
  rotate(identity: ClientIdentity): void {
    const old = this.#agent
    this.#agent = NodeClient.#agentFor(identity)
    setTimeout(() => old.destroy(), 2 * MINUTE_MS).unref()
  }

  /**
   * One call to the node. One that is safe to repeat (`repeatable`) is sent again while the node
   * doesn't answer, as long as its timeout leaves time to; a TLS failure is never retried, since
   * the same certificates fail the same way.
   */
  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    const deadline = performance.now() + (options.timeoutMs ?? 30_000)
    const delays = repeatable(method, path) ? RETRY_DELAYS_MS : []
    for (let attempt = 0; ; attempt++) {
      try {
        return await this.#once<T>(node, method, path, options, deadline - performance.now())
      } catch (error) {
        const delay = delays[attempt]
        if (
          !(error instanceof NodeUnreachable) ||
          error.kind === 'tls' ||
          delay === undefined ||
          performance.now() + delay >= deadline
        )
          throw error
        await sleep(delay)
      }
    }
  }

  #once<T>(
    node: NodeAddress,
    method: string,
    path: string,
    options: CallOptions,
    timeoutMs: number,
  ): Promise<T> {
    const { host, port } = splitAddress(node.apiAddress)
    const body = options.body === undefined ? null : Buffer.from(JSON.stringify(options.body))
    const headers: Record<string, string> = {
      accept: 'application/json',
      ...options.headers,
    }
    if (body !== null) headers['content-type'] = 'application/json'
    if (options.epoch !== undefined && options.epoch !== null) headers[EPOCH_HEADER] = String(options.epoch)
    return new Promise<T>((resolve, reject) => {
      const request = https.request(
        {
          host,
          port,
          method,
          path,
          agent: this.#agent,
          // The node's certificate names the node, not the address it was dialled at.
          servername: nodeName(node.id, this.#deployment),
          headers: body === null ? headers : { ...headers, 'content-length': String(body.length) },
        },
        (response) => {
          const chunks: Buffer[] = []
          response.on('data', (chunk: Buffer) => chunks.push(chunk))
          response.on('error', (error) => reject(unreachable(error)))
          response.on('end', () => {
            clearTimeout(timer)
            const text = Buffer.concat(chunks).toString('utf8')
            const status = response.statusCode ?? 0
            let parsed: unknown = null
            try {
              parsed = text === '' ? null : JSON.parse(text)
            } catch {
              parsed = null
            }
            if (status >= StatusCodes.OK && status < StatusCodes.MULTIPLE_CHOICES) return resolve(parsed as T)
            const error = (
              parsed as {
                error?: { code?: string; message?: string; details?: unknown }
              } | null
            )?.error
            reject(
              new NodeRefused(
                status,
                error?.code ?? `http_${status}`,
                error?.message ?? text.slice(0, 200),
                error?.details,
              ),
            )
          })
        },
      )
      const timer = setTimeout(() => {
        request.destroy(
          Object.assign(new Error(`no answer within ${Math.round(timeoutMs)} ms`), {
            code: 'TIMEOUT',
          }),
        )
      }, timeoutMs)
      request.on('error', (error: NodeJS.ErrnoException) => {
        clearTimeout(timer)
        reject(unreachable(error))
      })
      if (body !== null) request.write(body)
      request.end()
    })
  }

  /**
   * The lines of a streamed answer (pryxd's NDJSON logs), as they arrive. Ends when the node
   * ends the stream or `signal` aborts.
   */
  async *lines(
    node: NodeAddress,
    path: string,
    signal: AbortSignal,
    options: CallOptions & { method?: string } = {},
  ): AsyncIterable<string> {
    if (options.timeoutMs !== undefined)
      signal = AbortSignal.any([signal, AbortSignal.timeout(options.timeoutMs)])
    const { host, port } = splitAddress(node.apiAddress)
    const response = await new Promise<import('node:http').IncomingMessage>((resolve, reject) => {
      const request = https.request(
        {
          host,
          port,
          method: options.method ?? 'GET',
          path,
          agent: this.#agent,
          servername: nodeName(node.id, this.#deployment),
          headers: {
            accept: 'application/x-ndjson',
            ...(options.body ? { 'content-type': 'application/json' } : {}),
            ...(options.epoch == null ? {} : { [EPOCH_HEADER]: String(options.epoch) }),
            ...options.headers,
          },
          signal,
        },
        resolve,
      )
      request.on('error', (error: NodeJS.ErrnoException) => reject(unreachable(error)))
      request.end(options.body ? JSON.stringify(options.body) : undefined)
    })
    if ((response.statusCode ?? 0) >= StatusCodes.MULTIPLE_CHOICES) {
      const chunks: Buffer[] = []
      for await (const chunk of response) chunks.push(chunk as Buffer)
      const text = Buffer.concat(chunks).toString('utf8')
      let error: { code?: string; message?: string } | undefined
      try {
        error = (JSON.parse(text) as { error?: { code?: string; message?: string } }).error
      } catch {
        error = undefined
      }
      throw new NodeRefused(
        response.statusCode ?? 0,
        error?.code ?? 'http_error',
        error?.message ?? text.slice(0, 200),
        null,
      )
    }
    yield* ndjsonLines(response, signal)
  }

  /**
   * What can be learned about a quiet node from outside it: does its API port take a connection,
   * and does pryxd answer on it? A refusal means the host is up and pryxd is not; a timeout,
   * that the host or the path to it is down.
   */
  async probe(node: NodeAddress, timeoutMs = 3000): Promise<Probe> {
    const started = performance.now()
    const tcp = await tcpProbe(node.apiAddress, timeoutMs)
    if (tcp !== 'open') return { tcp, api: null, ms: Math.round(performance.now() - started) }
    try {
      await this.call(node, 'GET', '/v1/health', { timeoutMs })
      return { tcp, api: 'ok', ms: Math.round(performance.now() - started) }
    } catch (error) {
      return {
        tcp,
        api: `error: ${(error as Error).message}`,
        ms: Math.round(performance.now() - started),
      }
    }
  }

  close(): void {
    this.#agent.destroy()
  }

  /** A file stream is sent once, including PUT: a partial upload is never silently replayed. */
  async transfer(
    node: NodeAddress,
    method: string,
    path: string,
    options: { epoch: number; signal: AbortSignal; body?: ReadableStream<Uint8Array> },
  ) {
    const { host, port } = splitAddress(node.apiAddress)
    const response = await new Promise<import('node:http').IncomingMessage>((resolve, reject) => {
      const request = https.request(
        {
          host,
          port,
          method,
          path,
          agent: this.#agent,
          servername: nodeName(node.id, this.#deployment),
          headers: { [EPOCH_HEADER]: String(options.epoch) },
          signal: options.signal,
        },
        resolve,
      )
      request.on('error', (error: NodeJS.ErrnoException) => reject(unreachable(error)))
      request.setTimeout(30 * SECOND_MS, () => request.destroy(new Error('Node file stream became idle')))
      if (options.body) {
        // Node and Bun describe the same WHATWG stream with different convenience methods.
        const input = Readable.fromWeb(options.body as unknown as Parameters<typeof Readable.fromWeb>[0])
        input.on('error', () => request.destroy(new Error('File upload interrupted')))
        request.once('close', () => input.destroy())
        input.pipe(request)
      } else request.end()
    })
    const headers = new Headers()
    for (const name of ['content-type', 'content-length', 'cache-control']) {
      const value = response.headers[name]
      if (typeof value === 'string') headers.set(name, value)
    }
    return new Response(
      response.statusCode === StatusCodes.NO_CONTENT
        ? null
        : (Readable.toWeb(response) as unknown as ReadableStream<Uint8Array>),
      { status: response.statusCode, headers },
    )
  }
}

/**
 * The non-blank lines of a streamed NDJSON body, as they arrive, the last one even without its
 * newline. The body is decoded as one UTF-8 stream, not chunk by chunk: a character whose bytes
 * two chunks split (a application colour code's '§', a player's name) arrives whole. Ends quietly
 * once `signal` aborts; any other error ends it with that error.
 */
export async function* ndjsonLines(body: Readable, signal: AbortSignal): AsyncIterable<string> {
  body.setEncoding('utf8')
  let rest = ''
  try {
    for await (const chunk of body) {
      rest += chunk as string
      let at = rest.indexOf('\n')
      while (at >= 0) {
        const line = rest.slice(0, at)
        rest = rest.slice(at + 1)
        if (line.trim() !== '') yield line
        at = rest.indexOf('\n')
      }
    }
  } catch (error) {
    if (!signal.aborted) throw error
  }
  if (rest.trim() !== '') yield rest
}

export interface Probe {
  tcp: 'open' | 'refused' | 'timeout' | 'unreachable'
  api: string | null
  ms: number
}

function tcpProbe(address: string, timeoutMs: number): Promise<Probe['tcp']> {
  const { host, port } = splitAddress(address)
  return new Promise((resolve) => {
    const socket = tcpConnect({ host, port })
    const done = (result: Probe['tcp']) => {
      socket.destroy()
      resolve(result)
    }
    socket.setTimeout(timeoutMs, () => done('timeout'))
    socket.on('connect', () => done('open'))
    socket.on('error', (error: NodeJS.ErrnoException) =>
      done(
        error.code === 'ECONNREFUSED' ? 'refused' : error.code === 'ETIMEDOUT' ? 'timeout' : 'unreachable',
      ),
    )
  })
}
