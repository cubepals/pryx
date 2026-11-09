// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real SQL registration and authenticated agent metadata preserve SSH address and revocation
 * semantics. The privileged OpenSSH/edge wake lifecycle remains in native deployment acceptance. */
import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { requestToken, sandboxToken } from '../../edge/src/sandbox-token.ts'
import type { PublicPort } from '../src/api-schemas.ts'
import { MINUTE_S, SECOND_MS } from '../src/durations.ts'
import { addresses } from '../src/edge-addresses.ts'
import { sawEdge } from '../src/edge-routed.ts'
import { EdgeService } from '../src/edge-service.ts'
import { decodeHandle } from '../src/handle.ts'
import { NativeSsh, sshRoutes } from '../src/native-ssh.ts'
import { lock, rows } from '../src/sql.ts'
import { publicContext } from './public-context.ts'

type Context = Awaited<ReturnType<typeof publicContext>>

/** A lease row and its reserved address, as registration writes them, without the agent round trip. */
async function lease(c: Context, sandbox: string, range: readonly [number, number], epochOffset = 0) {
  const box = await c.boxes.record(sandbox, 'default')
  if (!box.handle) throw new Error('Sandbox has no handle')
  const ref = decodeHandle(box.handle),
    slot = ref.agent?.spares[0]?.port,
    id = randomUUID()
  if (slot === undefined) throw new Error('Missing spare slot')
  await c.db.execute(
    sql`INSERT INTO sandbox_ssh(id,sandbox,project,epoch,slot,target_port,username,host_key,expires_at) VALUES(${id},${sandbox},'default',${ref.epoch + epochOffset},${slot},2222,'pryx','ssh-ed25519 AAAA',now()+(${MINUTE_S}*interval '1 second'))`,
  )
  const port: PublicPort = {
    name: `ssh-${id}`,
    port: slot,
    protocol: 'tcp',
    audience: 'public',
    front: 'port',
  }
  const assigned = (await addresses(c.db, sandbox, [port], 'sandboxes.test', range)).find(
    (a) => a.name === port.name,
  )
  if (!assigned) throw new Error('SSH address was not allocated')
  return { id, publicPort: assigned.public_port }
}

test('an expired or older-epoch SSH lease frees its public port after quarantine', async () => {
  const c = await publicContext()
  try {
    const made = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', idle: 60 })
    expect(made.status).toBe(StatusCodes.CREATED)
    const { id: sandbox } = (await made.json()) as { id: string }
    const range: readonly [number, number] = [20000, 20001]
    const expired = await lease(c, sandbox, range),
      obsolete = await lease(c, sandbox, range, -1)
    expect([expired.publicPort, obsolete.publicPort].sort()).toEqual([20000, 20001])
    await c.db.execute(sql`UPDATE sandbox_ssh SET expires_at=now() WHERE id=${expired.id}`)
    expect(await sshRoutes(c.db, await c.boxes.record(sandbox, 'default'))).toEqual([])
    const retired = await rows<{ name: string }>(
      c.db,
      sql`SELECT name FROM edge_addresses WHERE sandbox=${sandbox} AND retired_at IS NOT NULL ORDER BY name`,
    )
    expect(retired.map((r) => r.name).sort()).toEqual([`ssh-${expired.id}`, `ssh-${obsolete.id}`].sort())
    // The ports stay quarantined until the cleanup's ten minutes have passed.
    await expect(lease(c, sandbox, range)).rejects.toThrow('exhausted')
    await c.db.execute(
      sql`UPDATE edge_addresses SET retired_at=retired_at-(${10 * MINUTE_S + 1}*interval '1 second') WHERE sandbox=${sandbox} AND retired_at IS NOT NULL`,
    )
    const next = await lease(c, sandbox, range)
    expect(range).toContain(next.publicPort)
    expect(await sshRoutes(c.db, await c.boxes.record(sandbox, 'default'))).toHaveLength(1)
  } finally {
    await c.close()
  }
})

test('a user port named like an SSH address keeps its route beside a lease', async () => {
  const c = await publicContext()
  try {
    const made = await c.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      idle: 60,
      ports: [{ name: 'ssh-app', port: 8080, audience: 'public' }],
    })
    expect(made.status).toBe(StatusCodes.CREATED)
    const { id: sandbox } = (await made.json()) as { id: string }
    const held = await lease(c, sandbox, [20000, 20100])
    const listed = (await new EdgeService(c.db, c.boxes, 'sandboxes.test').routes()).routes
      .filter((r) => r.sandbox === sandbox)
      .map((r) => ({ id: r.id, ssh: Boolean(r.ssh) }))
    expect(listed).toHaveLength(2)
    expect(listed).toEqual(
      expect.arrayContaining([
        { id: `${sandbox}:ssh-${held.id}`, ssh: true },
        { id: `${sandbox}:ssh-app`, ssh: false },
      ]),
    )
  } finally {
    await c.close()
  }
})

test('SSH registration returns a bare IPv6 host and revocation removes its route', async () => {
  let reachedRouteLock: (() => void) | undefined
  let releaseRouteLock: (() => void) | undefined
  const c = await publicContext({
      logQuery(_query, parameters) {
        if (parameters[0] === 'edge-route-list') reachedRouteLock?.()
      },
    }),
    directory = await mkdtemp(join(tmpdir(), 'pryx-ssh-public-fixture-'))
  let server: ReturnType<typeof Bun.serve> | undefined
  try {
    const generated = Bun.spawn(
      ['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-C', '', '-f', join(directory, 'host')],
      {
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )
    expect(await generated.exited).toBe(0)
    const hostKey = (await readFile(join(directory, 'host.pub'), 'utf8')).trim()
    const made = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', idle: 60 })
    expect(made.status).toBe(StatusCodes.CREATED)
    const box = (await made.json()) as { id: string }
    const peer = c.peers.get(box.id)
    if (!peer) throw new Error('Missing authenticated agent peer')
    const id = randomUUID(),
      slot = peer.route.agent?.manifest?.spares[0]?.port
    if (slot === undefined) throw new Error('Missing spare slot')
    const calls: string[] = []
    server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch(request) {
        if (!sandboxToken(requestToken(request), peer.route))
          return new Response(null, { status: StatusCodes.UNAUTHORIZED })
        const path = new URL(request.url).pathname
        calls.push(`${request.method} ${path}`)
        if (path.endsWith('/_pryx/health')) return Response.json({ status: 'ok' })
        if (path.endsWith(`/_pryx/ssh/${id}`))
          return Response.json({
            id,
            port: 2222,
            slot,
            user: 'pryx',
            hostKey,
            expiresAt: Math.floor(Date.now() / SECOND_MS) + 30,
            running: true,
          })
        if (path.endsWith('/_pryx/ssh') && request.method === 'DELETE')
          return new Response(null, { status: StatusCodes.NO_CONTENT })
        return new Response(null, { status: StatusCodes.NOT_FOUND })
      },
    })
    c.agents.localOrigin = server.url.origin
    const service = new NativeSsh(c.db, c.boxes, c.agents, 'sandboxes.test')
    const edge = new EdgeService(c.db, c.boxes, 'sandboxes.test', undefined, c.agents)
    const prior = await edge.routes(true)
    await sawEdge(c.db, 'ssh-registration', prior.version)
    const locked = Promise.withResolvers<void>(),
      release = Promise.withResolvers<void>(),
      polling = Promise.withResolvers<void>()
    releaseRouteLock = () => release.resolve()
    const holding = c.db.transaction(async (transaction) => {
      await lock(transaction, 'edge-route-list')
      locked.resolve()
      await release.promise
    })
    await locked.promise
    reachedRouteLock = () => polling.resolve()
    const stalePoll = edge.routes(true)
    await polling.promise
    reachedRouteLock = undefined
    let issued = false
    const registration = service
      .register(box.id, 'default', { id }, `http://[::1]:8080/v1/sandboxes/${box.id}/ssh`)
      .then((lease) => {
        issued = true
        return lease
      })
    for (let attempt = 0; attempt < 100; attempt++) {
      if ((await sshRoutes(c.db, await c.boxes.record(box.id, 'default'))).length) break
      await Bun.sleep(SECOND_MS / 100)
    }
    release.resolve()
    await holding
    expect((await stalePoll).version).toBe(prior.version)
    await Bun.sleep(SECOND_MS / 20)
    expect(issued).toBe(false)
    const listed = await edge.routes(true)
    expect(listed.version).toBeGreaterThan(prior.version)
    await sawEdge(c.db, 'ssh-registration', prior.version)
    await Bun.sleep(SECOND_MS / 20)
    expect(issued).toBe(false)
    await sawEdge(c.db, 'ssh-registration', listed.version)
    const lease = await registration
    expect(lease.host).toBe('::1')
    expect(lease.hostKey).toBe(hostKey)
    expect((await sshRoutes(c.db, await c.boxes.record(box.id, 'default')))[0]?.ssh).toEqual({
      targetPort: 2222,
      session: id,
    })
    expect(calls.some((call) => call.endsWith(`/_pryx/ssh/${id}`))).toBe(true)
    await service.revoke(box.id, 'default', 'http://[::1]:8080')
    expect(await sshRoutes(c.db, await c.boxes.record(box.id, 'default'))).toEqual([])
    await expect(
      service.register(box.id, 'default', { id, key: 'private-key-not-accepted' }, 'http://[::1]:8080'),
    ).rejects.toThrow()
  } finally {
    releaseRouteLock?.()
    server?.stop(true)
    await c.close()
    await rm(directory, { recursive: true, force: true })
  }
})
