// SPDX-License-Identifier: FSL-1.1-ALv2
/** OpenSSH receives the per-call private key over ssh-add stdin. Only a socket and pinned
 * public host key use temporary files; the lease and local agent are revoked on exit. */
import { spawn } from 'node:child_process'
import { access, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as pause } from 'node:timers/promises'
import type { Pryx } from '@cubepals/pryx'
import { SECOND_MS } from '../../control/src/durations.ts'

export async function connectSsh(lease: import('@cubepals/pryx').SshAccess, args: string[]) {
  const directory = await mkdtemp(join(tmpdir(), 'pryx-ssh-')),
    socket = join(directory, 'agent'),
    known = join(directory, 'known_hosts')
  const agent = spawn('ssh-agent', ['-D', '-a', socket], { stdio: 'ignore' })
  const finished = new Promise<void>((resolve) => {
    agent.once('error', resolve)
    agent.once('exit', () => resolve())
  })
  try {
    await writeFile(known, `[${lease.host}]:${lease.port} ${lease.hostKey}\n`, { mode: 0o600 })
    let ready = false
    for (let attempt = 0; attempt < 100; attempt++) {
      if (agent.exitCode !== null) break
      try {
        await access(socket)
        ready = true
        break
      } catch {
        await pause(SECOND_MS / 100)
      }
    }
    if (!ready) throw new Error('Local ssh-agent did not become ready')
    const add = spawn('ssh-add', ['-'], {
      env: { ...process.env, SSH_AUTH_SOCK: socket },
      stdio: ['pipe', 'ignore', 'ignore'],
    })
    const added = new Promise<number | null>((resolve, reject) => {
      add.once('error', reject)
      add.once('exit', resolve)
    })
    add.stdin.end(lease.key)
    if ((await added) !== 0) throw new Error('OpenSSH did not accept the in-memory key')
    const session = spawn(
      'ssh',
      [
        '-F',
        '/dev/null',
        '-p',
        String(lease.port),
        '-o',
        `IdentityAgent=${socket}`,
        '-o',
        'IdentityFile=none',
        '-o',
        'StrictHostKeyChecking=yes',
        '-o',
        `UserKnownHostsFile=${known}`,
        '-o',
        'GlobalKnownHostsFile=/dev/null',
        '-o',
        'ForwardAgent=no',
        `${lease.user}@${lease.host}`,
        ...args,
      ],
      { stdio: 'inherit' },
    )
    return await new Promise<number>((resolve, reject) => {
      session.once('error', reject)
      session.once('exit', (code) => resolve(code ?? 1))
    })
  } finally {
    agent.kill('SIGTERM')
    await finished
    await rm(directory, { recursive: true, force: true })
  }
}

export async function ssh(client: Pryx, id: string, args: string[]) {
  const box = await client.sandbox(id),
    lease = await box.ssh()
  try {
    process.exitCode = await connectSsh(lease, args)
  } finally {
    await box.ssh.revoke()
  }
}
