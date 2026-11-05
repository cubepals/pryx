// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Sandbox and one-shot run commands use only the public SDK and caller-provided credentials. */

import { readFileSync, writeFileSync } from 'node:fs'
import type { Duration } from '@cubepals/pryx'
import { type MakeRequest, Pryx, seconds } from '@cubepals/pryx'
import { SDK_DEFAULTS } from '../../sdk/typescript/src/generated/defaults.ts'
import { readClientConfig } from './client-config.ts'
import { copy, terminal } from './sandbox-data.ts'

export function publicClient() {
  const saved = process.env.PRYX_API_KEY ? undefined : readClientConfig()
  const baseUrl = process.env.PRYX_API_URL ?? saved?.url ?? 'http://127.0.0.1:8082'
  const apiKey = process.env.PRYX_API_KEY ?? (saved?.url === baseUrl ? saved.key : undefined)
  if (!apiKey) throw new Error('Run pryx up or set PRYX_API_URL and PRYX_API_KEY')
  return new Pryx({ baseUrl, apiKey })
}
export function option(args: string[], name: string) {
  const at = args.indexOf(name)
  if (at < 0) return undefined
  const value = args[at + 1]
  if (!value || value.startsWith('--')) throw new Error(`${name} needs a value`)
  return value
}
const duration = (value: string) =>
  seconds(/^\d+(?:\.\d+)?$/.test(value) ? Number(value) : (value as Duration))
function makeRequest(args: string[]): MakeRequest {
  const spec = option(args, '--spec')
  if (spec) return JSON.parse(readFileSync(spec, 'utf8')) as MakeRequest
  const snapshot = option(args, '--from-snapshot')
  if (snapshot) return { snapshot }
  const image = option(args, '--image')
  const ttl = option(args, '--ttl'),
    idle = option(args, '--idle')
  const units = option(args, '--units')
  const network = option(args, '--network'),
    spares = option(args, '--spare-ports')
  if (network && network !== 'internet' && network !== 'none') throw new Error('network is internet or none')
  const packages = args.flatMap((value, index) =>
    value === '--package' ? [option(args.slice(index), '--package') as string] : [],
  )
  return {
    ...(packages.length ? { packages } : {}),
    ...(image ? { image } : {}),
    ...(option(args, '--isolation')
      ? { isolation: option(args, '--isolation') as MakeRequest['isolation'] }
      : {}),
    ...(network === 'internet' || network === 'none' ? { network } : {}),
    ...(spares ? { sparePorts: Number(spares) } : {}),
    ...(units ? { units: Number(units) } : {}),
    entrypoint: [...SDK_DEFAULTS.entrypoint],
    ...(ttl ? { ttl: duration(ttl) } : {}),
    ...(idle ? { idle: duration(idle) } : {}),
    start: !args.includes('--stopped'),
  }
}
export async function execCommand(
  client: Pryx,
  id: string,
  command: string[],
  options: { timeout?: number; key?: string } = {},
) {
  const box = await client.sandbox(id)
  {
    const run = box.run(command, {
      ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
      idempotencyKey: options.key,
    })
    for await (const chunk of run.stream()) {
      const destination = chunk.stream === 'stdout' ? process.stdout : process.stderr
      if (!destination.write(chunk.data))
        await new Promise<void>((resolve) => destination.once('drain', resolve))
    }
    return (await run).code ?? 1
  }
}
export async function sandbox(args: string[]): Promise<unknown> {
  const client = publicClient()
  const [verb = 'ls', id, ...rest] = args
  if (verb === 'create')
    return client.make(makeRequest(args), option(args, '--idempotency-key') ?? crypto.randomUUID())
  if (verb === 'ls') return client.list()
  if (verb === 'cp') {
    if (!id || !rest[0]) throw new Error('sandbox cp needs a source and destination')
    await copy(client, id, rest[0])
    return
  }
  if (!id) throw new Error(`sandbox ${verb} needs an id`)
  if (verb === 'get') return client.get(id)
  if (verb === 'mcp-url') {
    console.log(await (await client.sandbox(id)).mcpUrl())
    return
  }
  if (verb === 'terminal') {
    await terminal(await client.sandbox(id))
    return
  }
  if (verb === 'expose') {
    if (!rest[0]) throw new Error('sandbox expose needs a target port')
    return (await client.sandbox(id)).expose(Number(rest[0]))
  }
  if (verb === 'unexpose') {
    await (await client.sandbox(id)).unexpose(Number(rest[0]))
    return
  }
  if (verb === 'network') {
    const mode = rest[0]
    if (mode !== 'internet' && mode !== 'none') throw new Error('network is internet or none')
    return (await client.sandbox(id)).network(mode)
  }
  if (verb === 'rm') return client.delete(id)
  if (verb === 'snapshot') return client.snapshot(id)
  if (verb === 'fork') return client.fork(id, {}, option(args, '--idempotency-key') ?? crypto.randomUUID())
  if (verb === 'endpoints') return client.endpoints(id)
  if (verb === 'events') return client.events(id, option(rest, '--since'))
  if (verb === 'start' || verb === 'stop' || verb === 'sleep' || verb === 'wake' || verb === 'rest')
    return client.action(id, verb)
  if (verb === 'exec') {
    const split = rest.indexOf('--')
    const options = split < 0 ? [] : rest.slice(0, split)
    const timeout = option(options, '--timeout'),
      key = option(options, '--idempotency-key')
    process.exitCode = await execCommand(client, id, split < 0 ? rest : rest.slice(split + 1), {
      ...(timeout ? { timeout: duration(timeout) } : {}),
      ...(key ? { key } : {}),
    })
    return
  }
  if (verb === 'logs') {
    for await (const line of client.logs(id, {
      follow: rest.includes('--follow'),
      ...(option(rest, '--tail') ? { tail: Number(option(rest, '--tail')) } : {}),
    }))
      console.log(JSON.stringify(line))
    return
  }
  throw new Error(`Unknown sandbox command: ${verb}`)
}
export async function run(args: string[]) {
  const split = args.indexOf('--')
  if (split < 0 || split === args.length - 1)
    throw new Error('run [--image <image>] [--isolation container|process|wasm] [--snapshot] -- <command...>')
  const client = publicClient()
  const made = await client.make(
    {
      ...makeRequest(args.slice(0, split)),
      entrypoint: [...SDK_DEFAULTS.entrypoint],
    },
    crypto.randomUUID(),
  )
  try {
    const timeout = option(args.slice(0, split), '--timeout')
    const code = await execCommand(
      client,
      made.id,
      args.slice(split + 1),
      timeout ? { timeout: duration(timeout) } : {},
    )
    if (args.slice(0, split).includes('--snapshot')) {
      const snapshot = await client.snapshot(made.id)
      const retained = await client.fork(snapshot.id, { start: false })
      await client.action(retained.id, 'rest')
      const kept = (await client.snapshots(retained.id)).snapshots[0]
      console.error(JSON.stringify({ snapshot: kept?.id, sandbox: retained.id }))
      const output = option(args.slice(0, split), '--snapshot-output')
      if (output && kept) writeFileSync(output, kept.id + '\n', { mode: 0o600 })
    }
    process.exitCode = code
  } finally {
    await client.delete(made.id)
  }
}
