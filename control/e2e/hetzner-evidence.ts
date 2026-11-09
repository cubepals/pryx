// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What the live Hetzner test says about itself: timestamped progress, named waits, a preflight of
 * what the rented box will download before a server is paid for, the box's and control's own
 * account of a failure, and cleanup that outlasts Hetzner's "still in use" answers. It prints no
 * token, join line or key: join tokens in the box's output are redacted. Not for the test's
 * assertions, which stay in hetzner.e2e.test.ts.
 */
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { BOOTSTRAP_OFFER_BYTES } from '../../cli/src/join.ts'
import { pinnedJoinDownloads } from '../../cli/src/join-download.ts'
import { verifyBootstrapOffer } from '../../cli/src/join-trust.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { caSha256, JOIN_TOKEN_PREFIX } from '../src/join-token.ts'
import type { startControl } from '../src/main.ts'
import type { ProviderClient } from '../src/providers/client.ts'
import { ProviderError } from '../src/providers/provider.ts'
import { rows } from '../src/sql.ts'

type Control = Awaited<ReturnType<typeof startControl>>
/** Lines kept of each piece of box output. */
const EVIDENCE_LINES = 200
const BOX_COMMANDS = [
  'cloud-init status --long',
  `tail -n ${EVIDENCE_LINES} /var/log/cloud-init-output.log`,
  'journalctl -u pryxd --no-pager -n 100',
  'systemctl status pryxd --no-pager --full',
]
/** Join tokens in text, replaced so none is printed. */
export const redact = (text: string) =>
  text.replace(
    new RegExp(`${JOIN_TOKEN_PREFIX.replace('.', '\\.')}[A-Za-z0-9_-]+`, 'g'),
    `${JOIN_TOKEN_PREFIX}<redacted>`,
  )
const tail = (text: string) => redact(text).trimEnd().split('\n').slice(-EVIDENCE_LINES).join('\n')
const message = (error: unknown) => (error instanceof Error ? error.message : String(error))

export const progress = (stage: string, detail = '') =>
  console.info(`hetzner-e2e ${new Date().toISOString()} ${stage}${detail ? `: ${detail}` : ''}`)

/** Polls once a second; a timeout names the wait, its budget and, when given, what was last seen. */
export async function wait<T>(
  label: string,
  run: () => Promise<T | null>,
  minutes = 10,
  lastSeen: () => string = () => '',
): Promise<T> {
  const started = performance.now(),
    deadline = started + minutes * MINUTE_MS
  progress(`waiting for ${label}`, `up to ${minutes} min`)
  while (performance.now() < deadline) {
    const result = await run()
    if (result) {
      progress(`${label} reached`, `after ${Math.round((performance.now() - started) / SECOND_MS)} s`)
      return result
    }
    await pause(SECOND_MS)
  }
  const seen = lastSeen()
  throw new Error(`Timed out after ${minutes} min waiting for ${label}${seen ? `; last seen: ${seen}` : ''}`)
}

/**
 * The box's bootstrap installs the CLI from the URL in its own line, then `pryx node join` takes
 * control's bootstrap offer only with a release signature. Both are checked here, the way the box
 * does, so a run that could never join fails before a server is rented.
 */
export async function preflight(control: Control, caPem: string) {
  const installer = /https:\/\/\S+?\/install\.sh/.exec(control.hosts.bootstrap('preflight').line)?.[0]
  if (!installer) throw new Error('Preflight: the provider bootstrap line names no install.sh')
  const answer = await fetch(installer, { method: 'HEAD', redirect: 'follow' })
  if (!answer.ok)
    throw new Error(
      `Preflight: the rented box's bootstrap downloads ${installer}, which answers HTTP ${answer.status}; ` +
        'with no published release of this version the box cannot install the CLI or join. No server was rented.',
    )
  const downloads = await pinnedJoinDownloads(control.config.joinUrl, caSha256(caPem))
  try {
    verifyBootstrapOffer(
      JSON.parse((await downloads.get('bootstrap.json', BOOTSTRAP_OFFER_BYTES)).toString()),
    )
  } catch (error) {
    throw new Error(
      `Preflight: the box's pryx node join would refuse control's bootstrap offer (${message(error)}); ` +
        'control needs a release-signed pryxd and its .bootstrap.sig. No server was rented.',
    )
  }
  progress('preflight passed', `installer ${installer} answers, bootstrap offer is release-signed`)
}

/** The box's own account, over SSH, or why it can't be asked. */
export function boxEvidence(ssh: string[], ip: string) {
  const run = (command: string) =>
    spawnSync(ssh[0] ?? 'ssh', [...ssh.slice(1), `root@${ip}`, command], {
      encoding: 'utf8',
      timeout: 30 * SECOND_MS,
      maxBuffer: 16 * 1024 * 1024,
    })
  const probe = run('true')
  if (probe.status !== 0) {
    console.error(
      `BOX root@${ip} port 22 is not reachable over SSH (exit ${probe.status ?? probe.signal}): ${probe.error ? message(probe.error) : tail(probe.stderr ?? '')}`,
    )
    return
  }
  for (const command of BOX_COMMANDS) {
    const result = run(command)
    console.error(
      `BOX $ ${command} (exit ${result.status ?? result.signal})\n${tail(`${result.stdout}${result.stderr}`)}`,
    )
  }
}

/** Control's view of the acquisition and of the node its token enrolled, if any. */
export async function controlEvidence(control: Control, acquisition: string) {
  const view = await control.hosts.view(acquisition)
  console.error('CONTROL acquisition', JSON.stringify(view))
  const record = await control.hosts.record(acquisition)
  const tokens = await rows(
    control.db,
    sql`SELECT used_by, expires_at FROM fleet_node_tokens WHERE id=${record.token_id}`,
  )
  console.error('CONTROL enrollment token', JSON.stringify(tokens))
  const nodes = await rows(
    control.db,
    sql`SELECT n.id, n.name, n.lifecycle, n.last_heartbeat_at, n.runtime_up, n.reconciled, n.duplicate_seen_at FROM fleet_nodes n JOIN fleet_node_tokens t ON t.used_by=n.id WHERE t.id=${record.token_id}`,
  )
  console.error('CONTROL enrolled node', JSON.stringify(nodes))
}

export function tunnelEvidence(tunnel: Bun.Subprocess | undefined, log: string) {
  if (!tunnel) {
    console.error('TUNNEL was never opened')
    return
  }
  console.error(
    `TUNNEL ${tunnel.exitCode === null ? 'still running' : `exited ${tunnel.exitCode}`}\n${existsSync(log) ? tail(readFileSync(log, 'utf8')) : ''}`,
  )
}

/** A paid server: deleted and seen gone, or this throws. */
export async function deleteServer(api: ProviderClient, id: string) {
  await api.request(`/servers/${id}`, 'DELETE')
  await wait(
    `server ${id} gone`,
    async () => {
      try {
        await api.request(`/servers/${id}`)
        return null
      } catch (error) {
        return error instanceof ProviderError && error.status === StatusCodes.NOT_FOUND ? true : null
      }
    },
    3,
  )
}

/**
 * A firewall, network or key stays "in use" (HTTP 422) for a while after its server is deleted. Retries
 * every 5 s for up to 2 min, then warns with the id for a person to delete; never throws.
 */
export async function deleteWhenFree(api: ProviderClient, path: string, id: number) {
  const deadline = performance.now() + 2 * MINUTE_MS
  for (;;) {
    try {
      await api.request(`/${path}/${id}`, 'DELETE')
      progress('deleted', `${path}/${id}`)
      return
    } catch (error) {
      const inUse = error instanceof ProviderError && error.status === StatusCodes.UNPROCESSABLE_ENTITY
      if (!inUse || performance.now() >= deadline) {
        console.warn(`WARNING: Hetzner ${path}/${id} was not deleted (${message(error)}); delete it by hand`)
        return
      }
      await pause(5 * SECOND_MS)
    }
  }
}
