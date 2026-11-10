// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Sandbox and one-shot run commands use only the public SDK and caller-provided credentials. */

import { readFileSync, writeFileSync } from 'node:fs'
import type { Duration } from '@cubepals/pryx'
import { type MakeRequest, Pryx } from '@cubepals/pryx'
import { seconds } from '../../sdk/typescript/src/duration.ts'
import { SDK_DEFAULTS } from '../../sdk/typescript/src/generated/defaults.ts'
import { Transport } from '../../sdk/typescript/src/transport.ts'
import { readClientConfig } from './client-config.ts'
import { diagnosticFetch } from './http-diagnostics.ts'
import { copy, terminal } from './sandbox-data.ts'
import { field } from './sandbox-field.ts'

export function publicCredentials() {
  const saved = process.env.PRYX_API_KEY ? undefined : readClientConfig()
  const baseUrl = process.env.PRYX_API_URL ?? saved?.url ?? 'http://127.0.0.1:8082'
  const apiKey = process.env.PRYX_API_KEY ?? (saved?.url === baseUrl ? saved.key : undefined)
  if (!apiKey) throw new Error('Run pryx up or set PRYX_API_URL and PRYX_API_KEY')
  return { url: baseUrl, key: apiKey }
}
export function publicClient() {
  return new Pryx({
    ...publicCredentials(),
    ...(process.env.PRYX_HTTP_DIAGNOSTICS === '1'
      ? { fetch: diagnosticFetch((event) => console.error(JSON.stringify(event))) }
      : {}),
  })
}
export function publicTransport() {
  return new Transport(publicCredentials())
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
    ...(option(args, '--name') ? { name: option(args, '--name') } : {}),
    ...(option(args, '--workdir') ? { workdir: option(args, '--workdir') } : {}),
    ...(option(args, '--user') ? { user: option(args, '--user') } : {}),
    ...(args.includes('--auto-delete') ? { autoDelete: true } : {}),
    ...(option(args, '--env') ? { env: JSON.parse(option(args, '--env') as string) } : {}),
    ...(option(args, '--labels') ? { labels: JSON.parse(option(args, '--labels') as string) } : {}),
    ...(option(args, '--metadata') ? { metadata: JSON.parse(option(args, '--metadata') as string) } : {}),
    ...(option(args, '--egress') ? { egress: JSON.parse(option(args, '--egress') as string) } : {}),
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
  options: { timeout?: number; key?: string; stdin?: Uint8Array } = {},
) {
  const box = await client.sandbox(id)
  {
    const run = box.stream(command, {
      ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
      idempotencyKey: options.key,
      stdin: options.stdin,
    })
    for await (const chunk of run) {
      const destination = chunk.stream === 'stdout' ? process.stdout : process.stderr
      if (!destination.write(chunk.data))
        await new Promise<void>((resolve) => destination.once('drain', resolve))
    }
    return (await run.result()).code ?? 1
  }
}
export async function sandbox(args: string[]): Promise<unknown> {
  const client = publicClient()
  const [verb = 'ls', id, ...rest] = args
  if (verb === 'ssh') {
    if (!id) throw new Error('sandbox ssh needs an id')
    return (await import('./sandbox-ssh.ts')).ssh(client, id, rest)
  }
  if (verb === 'python') return (await import('./sandbox-kernel.ts')).python(client, args.slice(1))
  if (verb === 'create')
    return client.sandbox({
      ...makeRequest(args),
      idempotencyKey: option(args, '--idempotency-key') ?? crypto.randomUUID(),
    })
  if (verb === 'ls')
    return {
      sandboxes: await client.sandboxes({
        name: option(args, '--name'),
        state: option(args, '--state') as Parameters<Pryx['sandboxes']>[0] extends infer T
          ? T extends { state?: infer S }
            ? S
            : never
          : never,
        labels: option(args, '--labels') ? JSON.parse(option(args, '--labels') as string) : undefined,
      }),
    }
  if (verb === 'cp') {
    if (!id || !rest[0]) throw new Error('sandbox cp needs a source and destination')
    await copy(client, id, rest[0])
    return
  }
  if (!id) throw new Error(`sandbox ${verb} needs an id`)
  if (verb === 'get') return client.sandbox(id, { wake: false })
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
    return (await client.sandbox(id, { wake: false })).network(mode)
  }
  if (verb === 'rm') return (await client.sandbox(id, { wake: false })).delete()
  if (verb === 'snapshot') return (await client.sandbox(id, { wake: false })).snapshot()
  if (verb === 'fork')
    return client.sandbox({
      snapshot: id,
      idempotencyKey: option(args, '--idempotency-key') ?? crypto.randomUUID(),
    })
  if (verb === 'endpoints')
    return { endpoints: await (await client.sandbox(id, { wake: false })).endpoints() }
  if (verb === 'events') {
    for await (const event of (await client.sandbox(id, { wake: false })).events({
      since: option(rest, '--since'),
      follow: rest.includes('--follow'),
    }))
      console.log(JSON.stringify(event))
    return
  }
  if (verb === 'start' || verb === 'stop' || verb === 'sleep' || verb === 'wake' || verb === 'rest')
    return { sandbox: await (await client.sandbox(id, { wake: false }))[verb]() }
  if (verb === 'exec') {
    const split = rest.indexOf('--')
    const options = split < 0 ? [] : rest.slice(0, split)
    const timeout = option(options, '--timeout'),
      key = option(options, '--idempotency-key')
    process.exitCode = await execCommand(client, id, split < 0 ? rest : rest.slice(split + 1), {
      ...(timeout ? { timeout: duration(timeout) } : {}),
      ...(key ? { key } : {}),
      ...(options.includes('--stdin') ? { stdin: new Uint8Array(await Bun.stdin.arrayBuffer()) } : {}),
    })
    return
  }
  if (verb === 'logs') {
    for await (const line of (await client.sandbox(id)).logs({
      follow: rest.includes('--follow'),
      ...(option(rest, '--tail') ? { tail: Number(option(rest, '--tail')) } : {}),
    }))
      console.log(JSON.stringify(line))
    return
  }
  const extra = await field(await client.sandbox(id, { wake: false }), verb, rest)
  if (extra.handled) return extra.result
  throw new Error(`Unknown sandbox command: ${verb}`)
}
export async function run(args: string[]) {
  const split = args.indexOf('--')
  if (split < 0 || split === args.length - 1)
    throw new Error('run [--image <image>] [--isolation container|process|wasm] [--snapshot] -- <command...>')
  const client = publicClient()
  const made = await client.sandbox({
    ...makeRequest(args.slice(0, split)),
    idempotencyKey: crypto.randomUUID(),
    entrypoint: [...SDK_DEFAULTS.entrypoint],
  })
  try {
    const timeout = option(args.slice(0, split), '--timeout')
    const code = await execCommand(client, made.id, args.slice(split + 1), {
      ...(timeout ? { timeout: duration(timeout) } : {}),
      ...(args.slice(0, split).includes('--stdin')
        ? { stdin: new Uint8Array(await Bun.stdin.arrayBuffer()) }
        : {}),
    })
    if (args.slice(0, split).includes('--snapshot')) {
      const snapshot = await made.snapshot()
      const retained = await snapshot.fork({ start: false })
      await retained.rest()
      const kept = (await retained.snapshots())[0]
      console.error(JSON.stringify({ snapshot: kept?.id, sandbox: retained.id }))
      const output = option(args.slice(0, split), '--snapshot-output')
      if (output && kept) writeFileSync(output, `${kept.id}\n`, { mode: 0o600 })
    }
    process.exitCode = code
  } finally {
    await made.delete()
  }
}
