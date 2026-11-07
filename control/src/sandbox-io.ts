// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public exec, logs and file transport with feature-gated node attachment; sandbox lifecycle belongs to sandboxes.ts. */
import type { z } from 'zod'
import type { AgentForward } from './agent-forward.ts'
import { type Exec, execEvent } from './api-schemas.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { responseEvents } from './exec-events.ts'
import { decodeHandle } from './handle.ts'
import type { NodeClient } from './node-client.ts'
import { claimExec, complete, operation } from './operations.ts'
import type { RuntimeHandle } from './runtime-model.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import type { ExecResponse } from './wire.ts'

// Rust's MAX_EXEC_ARG (src/protocol/exec.rs) is 128 KiB. Base64 and the shell script must fit one argument.
export const FILE_LIMIT = 48 * 1024
export class SandboxIO {
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly nodes: NodeClient,
    readonly agents?: AgentForward,
  ) {}
  private async home(handle: RuntimeHandle) {
    return this.boxes.engine.home(handle)
  }
  async execEvents(
    id: string,
    project: string,
    body: Exec,
    key?: string,
    signal = new AbortController().signal,
    requestUrl = 'http://127.0.0.1',
  ): Promise<AsyncIterable<z.infer<typeof execEvent>>> {
    const row = await this.boxes.record(id, project)
    const address = row.handle ? await this.home(row.handle) : null
    if (this.agents && row.handle && decodeHandle(row.handle).agent) {
      if (row.state !== 'running') throw new SandboxError('not_running', 'Sandbox is not running')
      return this.agents.exec(id, project, body, key, signal, requestUrl)
    }
    // An older node keeps its exact request/response protocol and durable outcome handling.
    if (!address?.features?.includes('exec-stream'))
      return responseEvents(await this.exec(id, project, body, key))
    const op = await operation(this.db, project, `exec:${id}`, key, body)
    if (op.response) return responseEvents(op.response as ExecResponse)
    if (row.state !== 'running' || !row.handle)
      throw new SandboxError('not_running', 'Sandbox is not running')
    if (key) await claimExec(this.db, op.id, id)
    else await this.boxes.touch(id)
    const command = Object.keys(body.env).length
      ? ['env', '--', ...Object.entries(body.env).map(([k, v]) => `${k}=${v}`), ...body.command]
      : body.command
    const epoch = decodeHandle(row.handle).epoch
    const { db, nodes } = this
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
      let exited = false
      for await (const line of nodes.lines(
        address,
        `/v1/workloads/${encodeURIComponent(id)}/exec-stream`,
        signal,
        {
          method: 'POST',
          body: { command, timeoutSeconds: body.timeout },
          epoch,
          headers: key ? { 'Idempotency-Key': op.id } : {},
          timeoutMs: (body.timeout + 5) * SECOND_MS,
        },
      )) {
        const event = execEvent.parse(JSON.parse(line))
        if (event.type === 'error') {
          yield event
          return
        }
        if (event.type === 'stdout' || event.type === 'stderr') {
          result[event.type] += event.data
          if (event.type === 'stdout') result.stdoutTruncated ||= event.truncated
          else result.stderrTruncated ||= event.truncated
        } else {
          exited = true
          Object.assign(result, {
            exitCode: event.code,
            timedOut: event.timedOut,
            killed: event.killed,
            durationMs: event.durationMs,
          })
          if (key) await complete(db, op.id, result)
        }
        yield event
      }
      if (!exited) throw new SandboxError('operation_unknown', 'Exec stream ended without a result')
    })()
  }
  async exec(id: string, project: string, body: Exec, key?: string) {
    const op = await operation(this.db, project, `exec:${id}`, key, body)
    if (op.response) return op.response as ExecResponse
    const row = await this.boxes.record(id, project)
    if (row.state !== 'running' || !row.handle)
      throw new SandboxError('not_running', 'Sandbox is not running')
    const address = await this.home(row.handle)
    if (!address) throw new SandboxError('not_running', 'Sandbox has no running home')
    if (key) await claimExec(this.db, op.id)
    await this.boxes.touch(id)
    // env(1) is executed inside the container. Values are argv entries, never shell interpolation.
    const command = Object.keys(body.env).length
      ? ['env', '--', ...Object.entries(body.env).map(([k, v]) => `${k}=${v}`), ...body.command]
      : body.command
    const result = await this.nodes.call<ExecResponse>(
      address,
      'POST',
      `/v1/workloads/${encodeURIComponent(id)}/exec`,
      {
        epoch: decodeHandle(row.handle).epoch,
        headers: key ? { 'Idempotency-Key': op.id } : {},
        body: { command, timeoutSeconds: body.timeout },
        timeoutMs: (body.timeout + 5) * SECOND_MS,
      },
    )
    if (key) await complete(this.db, op.id, result)
    return result
  }
  async *logs(
    id: string,
    project: string,
    query: URLSearchParams,
    signal: AbortSignal,
    requestUrl = 'http://127.0.0.1',
  ) {
    const row = await this.boxes.record(id, project)
    if (!row.handle) throw new SandboxError('not_running', 'Sandbox has no local logs')
    if (this.agents && row.state === 'running' && decodeHandle(row.handle).agent) {
      yield* this.agents.logs(id, project, query, signal, requestUrl)
      return
    }
    const address = await this.home(row.handle)
    if (!address) throw new SandboxError('not_running', 'Sandbox has no local logs')
    yield* this.nodes.lines(address, `/v1/workloads/${encodeURIComponent(id)}/logs?${query}`, signal)
  }
  async file(id: string, project: string, path: string, data?: string) {
    const row = await this.boxes.record(id, project)
    if (
      !path ||
      path.startsWith('/') ||
      path.split('/').some((p) => p === '..' || p === '.' || !p) ||
      path.includes('\0')
    )
      throw new SandboxError(
        'invalid_request',
        'File path is relative to persistent storage without dot segments',
      )
    if (
      data !== undefined &&
      (Buffer.from(data, 'base64').length > FILE_LIMIT || !/^[A-Za-z0-9+/]*={0,2}$/.test(data))
    )
      throw new SandboxError('invalid_request', `File is base64 with at most ${FILE_LIMIT} decoded bytes`)
    // realpath follows existing parents; its result must stay under the persistent mount even through symlinks.
    const guard =
      'base=$(realpath "$1") || exit; parent=$(realpath "$base/$(dirname "$2")") || exit; case "$parent/" in "$base/"*) ;; *) exit 1;; esac; target="$parent/$(basename "$2")"; [ ! -L "$target" ] || exit 1; '
    const script =
      data === undefined
        ? guard + 'tar -C "$base" -cf - -- "$2" | base64'
        : guard + 'printf %s "$3" | base64 -d > "$target"'
    const result = await this.exec(id, project, {
      command: [
        'sh',
        '-c',
        script,
        'pryx-file',
        row.spec.storage.mountPath,
        path,
        ...(data === undefined ? [] : [data]),
      ],
      timeout: 30,
      env: {},
    })
    if (result.exitCode !== 0 || result.stdoutTruncated || result.stderrTruncated)
      throw new SandboxError(
        'invalid_request',
        'File command failed or exceeded the node output bound; image needs sh, realpath, tar and base64',
      )
    return data === undefined ? { archiveBase64: result.stdout.replaceAll(/\s/g, '') } : { written: true }
  }
}
