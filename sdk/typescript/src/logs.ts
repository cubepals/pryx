// SPDX-License-Identifier: FSL-1.1-ALv2
/** Decode bounded agent log history directly; public HTTP log forwarding is a separate protocol boundary. */
import type { components } from './generated/api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { ndjson, PryxError } from './transport.ts'

export type LogsOptions = { tail?: number; since?: string; follow?: boolean }
export type LogRecord =
  | { ts: string; stream: 'stdout' | 'stderr'; line: string }
  | { event: 'gap'; reason: string }
type Frame = components['schemas']['AgentLogFrame']
type Chunk = Extract<Frame, { type: 'chunk' }>['chunk']

export async function* logs(read: () => Promise<Response>, options: LogsOptions): AsyncGenerator<LogRecord> {
  if (options.tail !== undefined && (!Number.isSafeInteger(options.tail) || options.tail < 0))
    throw new TypeError('tail is a non-negative integer')
  const since = options.since === undefined ? Number.NEGATIVE_INFINITY : Date.parse(options.since)
  if (Number.isNaN(since)) throw new TypeError('since is a timestamp')
  const rest = { stdout: '', stderr: '' },
    at = { stdout: 0, stderr: 0 }
  const decoders = { stdout: new TextDecoder(), stderr: new TextDecoder() }
  function decode(chunk: Chunk): LogRecord[] {
    const stream = chunk.stream
    if (stream !== 'stdout' && stream !== 'stderr')
      throw new PryxError(0, 'agent_protocol', 'Unknown log stream')
    if (!rest[stream]) at[stream] = chunk.at
    rest[stream] += decoders[stream].decode(
      Uint8Array.from(atob(chunk.bytes), (c) => c.charCodeAt(0)),
      { stream: true },
    )
    const out: LogRecord[] = []
    let end = rest[stream].indexOf('\n')
    while (end >= 0 || rest[stream].length >= SDK_DEFAULTS.captureBytes) {
      const size = end >= 0 ? Math.min(end, SDK_DEFAULTS.captureBytes) : SDK_DEFAULTS.captureBytes
      if (at[stream] >= since)
        out.push({
          ts: new Date(at[stream]).toISOString(),
          stream,
          line: rest[stream].slice(0, size).replace(/\r$/, ''),
        })
      rest[stream] = rest[stream].slice(size + (end === size ? 1 : 0))
      at[stream] = chunk.at
      end = rest[stream].indexOf('\n')
    }
    return out
  }
  function flush(): LogRecord[] {
    const out: LogRecord[] = []
    for (const stream of ['stdout', 'stderr'] as const) {
      rest[stream] += decoders[stream].decode()
      if (rest[stream] && at[stream] >= since)
        out.push({ ts: new Date(at[stream]).toISOString(), stream, line: rest[stream] })
      rest[stream] = ''
    }
    return out
  }
  const response = await read()
  for await (const frame of ndjson<Frame>(response)) {
    if (frame.type === 'replay') {
      const entries = frame.chunks.flatMap(decode)
      if (!options.follow) entries.push(...flush())
      yield* options.tail === 0 ? [] : options.tail === undefined ? entries : entries.slice(-options.tail)
    } else if (frame.type === 'chunk') yield* decode(frame.chunk)
    else yield { event: 'gap', reason: 'Agent log history or subscriber output was truncated' }
  }
  if (options.follow) yield* flush()
}
