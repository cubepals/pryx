// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Existing public exec and log records are adapted from the direct agent through the edge.
 * Durable exec outcomes remain in control; direct SDK requests bypass control. */
import { randomUUID } from 'node:crypto'
import { StringDecoder } from 'node:string_decoder'
import { StatusCodes } from 'http-status-codes'
import type { z } from 'zod'
import type { components } from '../../sdk/typescript/src/generated/api.ts'
import type { Agents } from './agents.ts'
import type { Exec, execEvent } from './api-schemas.ts'
import { SECOND_MS } from './durations.ts'
import { responseEvents } from './exec-events.ts'
import { agentSchema } from './node-contract.ts'
import { claimExec, complete, operation } from './operations.ts'
import { SandboxError } from './sandboxes.ts'
import { SDK_DEFAULTS } from './sdk-defaults.ts'
import type { ExecResponse, LogRecord } from './wire.ts'

type AgentEvent = components['schemas']['AgentEvent']
type Frame = components['schemas']['AgentLogFrame']
type Chunk = Extract<Frame, { type: 'chunk' }>['chunk']
const eventSchema = agentSchema('Event'),
  frameSchema = agentSchema('Frame')

async function* frames(response: Response): AsyncGenerator<unknown> {
  if (!response.ok || !response.body) throw new SandboxError('operation_unknown', 'Agent request failed')
  const reader = response.body.getReader(),
    decoder = new TextDecoder()
  let pending = ''
  try {
    while (true) {
      const next = await reader.read()
      pending += decoder.decode(next.value, { stream: !next.done })
      if (pending.length > 2 * SDK_DEFAULTS.captureBytes)
        throw new SandboxError('operation_unknown', 'Agent frame exceeded its bound')
      let end = pending.indexOf('\n')
      while (end >= 0) {
        const line = pending.slice(0, end)
        pending = pending.slice(end + 1)
        if (line) yield JSON.parse(line)
        end = pending.indexOf('\n')
      }
      if (next.done) {
        if (pending.trim()) yield JSON.parse(pending)
        return
      }
    }
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

export class AgentForward {
  constructor(readonly agents: Agents) {}
  async exec(
    id: string,
    project: string,
    body: Exec,
    key: string | undefined,
    signal: AbortSignal,
    requestUrl: string,
  ): Promise<AsyncIterable<z.infer<typeof execEvent>>> {
    const { boxes } = this.agents,
      db = boxes.db
    const row = await boxes.record(id, project)
    if (row.state !== 'running') throw new SandboxError('not_running', 'Sandbox is not running')
    const op = await operation(db, project, `exec:${id}`, key, body)
    if (op.response) return responseEvents(op.response as ExecResponse)
    if (key) await claimExec(db, op.id, id)
    else await boxes.touch(id)
    const credential = await this.agents.token(id, project, requestUrl)
    const abort = AbortSignal.any([signal, AbortSignal.timeout((body.timeout + 5) * SECOND_MS)])
    const headers = {
      authorization: `Bearer ${credential.token}`,
      'content-type': 'application/json',
      'idempotency-key': key ? op.id : randomUUID(),
    }
    const start = await fetch(`${credential.url}/_pryx/processes`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ command: body.command, env: body.env, timeout: body.timeout }),
      signal: abort,
      redirect: 'manual',
    })
    if (start.status !== StatusCodes.CREATED)
      throw new SandboxError('operation_unknown', 'Agent did not confirm command start')
    const view = agentSchema('View').parse(await start.json()) as components['schemas']['AgentProcess']
    const response = await fetch(`${credential.url}/_pryx/processes/${encodeURIComponent(view.id)}/output`, {
      headers: { authorization: headers.authorization },
      signal: abort,
      redirect: 'manual',
    })
    return (async function* () {
      const result: ExecResponse = {
        stdout: '',
        stderr: '',
        stdoutTruncated: false,
        stderrTruncated: false,
        exitCode: null,
        timedOut: false,
        killed: false,
        durationMs: 0,
      }
      const decoders = { stdout: new StringDecoder('utf8'), stderr: new StringDecoder('utf8') }
      let exited = false
      for await (const raw of frames(response)) {
        const event = eventSchema.parse(raw) as AgentEvent
        if (event.type === 'output') {
          if (event.stream !== 'stdout' && event.stream !== 'stderr')
            throw new SandboxError('operation_unknown', 'Unknown agent output stream')
          const data = decoders[event.stream].write(Buffer.from(event.bytes, 'base64'))
          const room = Math.max(0, SDK_DEFAULTS.captureBytes - result[event.stream].length)
          result[event.stream] += data.slice(0, room)
          if (data.length > room) result[`${event.stream}Truncated`] = true
          if (data) yield { type: event.stream, data, truncated: result[`${event.stream}Truncated`] } as const
        } else if (event.type === 'truncated') {
          result.stdoutTruncated = true
          result.stderrTruncated = true
          for (const type of ['stdout', 'stderr'] as const) yield { type, data: '', truncated: true }
        } else {
          for (const type of ['stdout', 'stderr'] as const) {
            const data = decoders[type].end()
            if (data) {
              result[type] += data
              yield { type, data, truncated: result[`${type}Truncated`] }
            }
          }
          Object.assign(result, {
            exitCode: event.result.code,
            timedOut: event.result.timedOut,
            killed: event.result.killed,
            durationMs: event.result.durationMs,
          })
          result.stdoutTruncated ||= event.result.stdoutTruncated
          result.stderrTruncated ||= event.result.stderrTruncated
          if (key) await complete(db, op.id, result)
          exited = true
          yield {
            type: 'exit',
            code: result.exitCode,
            timedOut: result.timedOut,
            killed: result.killed,
            durationMs: result.durationMs,
          } as const
        }
      }
      if (!exited) throw new SandboxError('operation_unknown', 'Agent output ended without a result')
    })()
  }

  async *logs(id: string, project: string, query: URLSearchParams, signal: AbortSignal, requestUrl: string) {
    const credential = await this.agents.token(id, project, requestUrl),
      follow = query.get('follow') === 'true'
    const response = await fetch(
      `${credential.url}/_pryx/logs?${new URLSearchParams({ follow: String(follow) })}`,
      { headers: { authorization: `Bearer ${credential.token}` }, signal, redirect: 'manual' },
    )
    const decoder = new LogDecoder(),
      since = query.has('since') ? Date.parse(query.get('since') as string) : Number.NEGATIVE_INFINITY
    const include = (line: LogRecord) => !('line' in line) || Date.parse(line.ts ?? '') >= since
    for await (const raw of frames(response)) {
      const frame = frameSchema.parse(raw) as Frame
      if (frame.type === 'replay') {
        const lines = frame.chunks.flatMap((chunk) => decoder.chunk(chunk)).filter(include)
        if (!follow) lines.push(...decoder.flush().filter(include))
        const tail = query.has('tail') ? Number(query.get('tail')) : lines.length
        if (!Number.isSafeInteger(tail) || tail < 0)
          throw new SandboxError('invalid_request', 'tail must be a non-negative integer')
        for (const line of tail === 0 ? [] : lines.slice(-tail)) yield JSON.stringify(line)
      } else if (frame.type === 'chunk')
        for (const line of decoder.chunk(frame.chunk).filter(include)) yield JSON.stringify(line)
      else
        yield JSON.stringify({ event: 'gap', reason: 'Agent log history or subscriber output was truncated' })
    }
    if (follow) for (const line of decoder.flush().filter(include)) yield JSON.stringify(line)
  }
}

class LogDecoder {
  readonly decoders = { stdout: new StringDecoder('utf8'), stderr: new StringDecoder('utf8') }
  readonly pending = { stdout: '', stderr: '' }
  readonly at = { stdout: 0, stderr: 0 }
  chunk(chunk: Chunk): LogRecord[] {
    if (chunk.stream !== 'stdout' && chunk.stream !== 'stderr')
      throw new SandboxError('operation_unknown', 'Unknown log stream')
    const stream = chunk.stream,
      lines: LogRecord[] = []
    if (!this.pending[stream]) this.at[stream] = chunk.at
    this.pending[stream] += this.decoders[stream].write(Buffer.from(chunk.bytes, 'base64'))
    let end = this.pending[stream].indexOf('\n')
    while (end >= 0 || this.pending[stream].length >= SDK_DEFAULTS.captureBytes) {
      const length = end >= 0 ? Math.min(end, SDK_DEFAULTS.captureBytes) : SDK_DEFAULTS.captureBytes
      lines.push({
        ts: new Date(this.at[stream]).toISOString(),
        stream,
        line: this.pending[stream].slice(0, length).replace(/\r$/, ''),
      })
      this.pending[stream] = this.pending[stream].slice(length + (end === length ? 1 : 0))
      this.at[stream] = chunk.at
      end = this.pending[stream].indexOf('\n')
    }
    return lines
  }
  flush(): LogRecord[] {
    const lines: LogRecord[] = []
    for (const stream of ['stdout', 'stderr'] as const) {
      this.pending[stream] += this.decoders[stream].end()
      if (this.pending[stream])
        lines.push({ ts: new Date(this.at[stream]).toISOString(), stream, line: this.pending[stream] })
      this.pending[stream] = ''
    }
    return lines
  }
}
