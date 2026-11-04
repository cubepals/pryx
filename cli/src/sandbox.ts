// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Sandbox and one-shot run commands use only the public SDK and caller-provided credentials. */

import { readFileSync } from 'node:fs'

import { type MakeRequest, Pryx } from '@cubepals/pryx'

export function publicClient() {
  const apiKey = process.env.PRYX_API_KEY
  if (!apiKey) throw new Error('Set PRYX_API_KEY')
  return new Pryx({ baseUrl: process.env.PRYX_API_URL ?? 'http://127.0.0.1:8082', apiKey })
}
export function option(args: string[], name: string) {
  const at = args.indexOf(name)
  if (at < 0) return undefined
  const value = args[at + 1]
  if (!value || value.startsWith('--')) throw new Error(`${name} needs a value`)
  return value
}
function makeRequest(args: string[]): MakeRequest {
  const spec = option(args, '--spec')
  if (spec) return JSON.parse(readFileSync(spec, 'utf8')) as MakeRequest
  const image = option(args, '--image')
  if (!image) throw new Error('create needs --image <image> or --spec <json file>')
  const ttl = option(args, '--ttl'),
    idle = option(args, '--idle')
  return {
    image,
    ...(ttl ? { ttl: Number(ttl) } : {}),
    ...(idle ? { idle: Number(idle) } : {}),
    start: !args.includes('--stopped'),
  }
}
export async function execCommand(client: Pryx, id: string, command: string[]) {
  let code: number | null = null
  for await (const event of client.exec(id, { command }, crypto.randomUUID())) {
    if (event.type === 'stdout') process.stdout.write(event.data)
    else if (event.type === 'stderr') process.stderr.write(event.data)
    else code = event.code
  }
  return code ?? 1
}
export async function sandbox(args: string[]): Promise<unknown> {
  const client = publicClient()
  const [verb = 'ls', id, ...rest] = args
  if (verb === 'create')
    return client.make(makeRequest(args), option(args, '--idempotency-key') ?? crypto.randomUUID())
  if (verb === 'ls') return client.list()
  if (!id) throw new Error(`sandbox ${verb} needs an id`)
  if (verb === 'get') return client.get(id)
  if (verb === 'rm') return client.delete(id)
  if (verb === 'snapshot') return client.snapshot(id)
  if (verb === 'fork') return client.fork(id, {}, option(args, '--idempotency-key') ?? crypto.randomUUID())
  if (verb === 'endpoints') return client.endpoints(id)
  if (verb === 'events') return client.events(id, option(rest, '--since'))
  if (verb === 'start' || verb === 'stop' || verb === 'sleep' || verb === 'wake' || verb === 'rest')
    return client.action(id, verb)
  if (verb === 'exec') {
    process.exitCode = await execCommand(client, id, rest[0] === '--' ? rest.slice(1) : rest)
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
    throw new Error('run --image <image> [--snapshot] -- <command...>')
  const client = publicClient()
  const made = await client.make(
    {
      ...makeRequest(args.slice(0, split)),
      entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
    },
    crypto.randomUUID(),
  )
  try {
    const code = await execCommand(client, made.id, args.slice(split + 1))
    if (args.slice(0, split).includes('--snapshot')) {
      const snapshot = await client.snapshot(made.id)
      const retained = await client.fork(snapshot.id, { start: false })
      await client.action(retained.id, 'rest')
      const kept = (await client.snapshots(retained.id)).snapshots[0]
      console.error(JSON.stringify({ snapshot: kept?.id, sandbox: retained.id }))
    }
    process.exitCode = code
  } finally {
    await client.delete(made.id)
  }
}
