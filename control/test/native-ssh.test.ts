// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real SQL registration and authenticated agent metadata preserve SSH address and revocation
 * semantics. The privileged OpenSSH/edge wake lifecycle remains in native deployment acceptance. */
import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { requestToken, sandboxToken } from '../../edge/src/sandbox-token.ts'
import { SECOND_MS } from '../src/durations.ts'
import { NativeSsh, sshRoutes } from '../src/native-ssh.ts'
import { publicContext } from './public-context.ts'

test('SSH registration returns a bare IPv6 host and revocation removes its route', async () => {
  const c = await publicContext(),
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
    const lease = await service.register(
      box.id,
      'default',
      { id },
      `http://[::1]:8080/v1/sandboxes/${box.id}/ssh`,
    )
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
    server?.stop(true)
    await c.close()
    await rm(directory, { recursive: true, force: true })
  }
})
