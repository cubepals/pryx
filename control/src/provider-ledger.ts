// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Durable host acquisition identities and derived bootstrap secrets; provider credentials remain in the environment. */
import { createHmac } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { HOUR_S } from './durations.ts'
import { PRYX_VERSION } from './generated/release.ts'
import { type JoinPoint, joinCommand, joinToken } from './join-token.ts'
import { operation } from './operations.ts'
import type { Bootstrap, Host, Provider, Shape } from './providers/provider.ts'
import { ACQUIRE_LABEL, ProviderError } from './providers/provider.ts'
import { createToken } from './registry.ts'
import type { Sandboxes } from './sandboxes.ts'
import { event, jsonb, one, rows } from './sql.ts'
export interface Acquisition {
  id: string
  provider: string
  region: string
  shape: Shape
  state: string
  host: Host | null
  token_id: string | null
  cancel_requested: boolean
  node_id: string | null
  reason: string
  requested_at: string
  ready_at: string | null
  error: string | null
}
export class ProviderLedger {
  private tail: Promise<void> = Promise.resolve()
  readonly tasks = new Map<string, Promise<void>>()
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly providers: ReadonlyMap<string, Provider>,
    readonly join: JoinPoint,
    readonly sealKey: string,
    readonly allowedImages: string[],
  ) {}
  provider(name: string) {
    const p = this.providers.get(name)
    if (!p) throw new ProviderError('refused', 'Provider is not configured')
    return p
  }
  private tokenSecret(id: string) {
    return createHmac('sha256', this.sealKey)
      .update(`pryx-provider-enrollment-v1:${this.join.deployment}:${id}`)
      .digest('base64url')
  }
  bootstrap(id: string): Bootstrap {
    return {
      // The verifier comes from the independent project distribution, never the fleet endpoint.
      line:
        `installer=$(mktemp) && curl --proto '=https' --proto-redir '=https' -fsSL 'https://github.com/cubepals/pryx/releases/download/v${PRYX_VERSION}/install.sh' -o "$installer" && sh "$installer" && rm -f "$installer" && ` +
        joinCommand(this.join, joinToken(this.join, this.tokenSecret(id))),
      allowedImages: this.allowedImages,
    }
  }
  async record(id: string) {
    const a = await one<Acquisition>(this.db, sql`SELECT * FROM provider_acquisitions WHERE id=${id}`)
    if (!a) throw new ProviderError('not_found', 'No such acquisition')
    return a
  }
  async view(id: string) {
    const a = await this.record(id)
    return {
      id: a.id,
      provider: a.provider,
      region: a.region,
      shape: a.shape,
      state: a.state,
      host: a.host,
      node: a.node_id,
      requestedAt: a.requested_at,
      readyAt: a.ready_at,
      error: a.error,
    }
  }
  async acquire(region: string, providerName: string, shapeName: string, key: string, reason = 'operator') {
    const provider = this.provider(providerName)
    const op = await operation(this.db, this.join.deployment, 'provider-acquire', key, {
      region,
      provider: providerName,
      shape: shapeName,
    })
    if (!(await one(this.db, sql`SELECT 1 FROM provider_acquisitions WHERE id=${op.id}`))) {
      const shape = (await provider.shapes(region)).find((s) => s.name === shapeName)
      if (!shape) throw new ProviderError('refused', 'Shape is unavailable in the region')
      await this.db.execute(
        sql`INSERT INTO provider_acquisitions(id,provider,region,shape,reason) VALUES(${op.id},${providerName},${region},${jsonb(shape)},${reason}) ON CONFLICT DO NOTHING`,
      )
    }
    this.queue(op.id)
    return this.view(op.id)
  }
  queue(id: string) {
    if (this.tasks.has(id)) return
    const task = this.tail
      .then(() => this.process(id))
      .catch(async () => {
        await this.db.execute(
          sql`UPDATE provider_acquisitions SET state='uncertain',error='Control could not process bootstrap; inspect configuration' WHERE id=${id}`,
        )
      })
      .finally(() => this.tasks.delete(id))
    this.tail = task.catch(() => {})
    this.tasks.set(id, task)
  }
  async settle() {
    await Promise.allSettled([...this.tasks.values()])
  }
  async process(id: string) {
    await this.boxes.exclusive(`provider:${id}`, async () => {
      let a = await this.record(id)
      const provider = this.provider(a.provider)
      if (['ready', 'released', 'releasing', 'refused', 'cancelled'].includes(a.state)) return
      if (a.cancel_requested) {
        await this.cancelLocked(a)
        return
      }
      if (a.node_id && a.host) return
      const bootstrap = this.bootstrap(id)
      if (!a.token_id) {
        const token = await createToken(this.db, {
          regionKey: a.region,
          ttlSeconds: HOUR_S,
          createdBy: 'provider',
          identity: { id, secret: this.tokenSecret(id) },
          labels: { [ACQUIRE_LABEL]: a.id, 'pryx.provider': a.provider },
        })
        await this.db.execute(sql`UPDATE provider_acquisitions SET token_id=${token.id} WHERE id=${id}`)
        a = { ...a, token_id: token.id }
      }
      try {
        let host = a.host
        if (a.state === 'executing' || a.state === 'uncertain') {
          host = (await provider.list(a.id)).find((h) => h.requestKey === a.id) ?? null
          if (!host) {
            await this.db.execute(
              sql`UPDATE provider_acquisitions SET state='uncertain',error='No matching host visible; automatic create retry is blocked' WHERE id=${id}`,
            )
            return
          }
        }
        if (!host) {
          await this.db.execute(sql`UPDATE provider_acquisitions SET state='executing' WHERE id=${id}`)
          await event(this.db, 'provider.acquire', {
            data: { id, provider: a.provider, region: a.region, shape: a.shape, reason: a.reason },
          })
          host = await provider.acquire(a.region, a.shape, a.id, bootstrap)
        }
        await this.db.execute(
          sql`UPDATE provider_acquisitions SET host=${jsonb(host)},state=CASE WHEN state='ready' THEN state ELSE 'waiting' END,error=NULL,acquired_at=COALESCE(acquired_at,now()) WHERE id=${id}`,
        )
        const enrolled = await one<{ id: string }>(
          this.db,
          sql`SELECT n.id FROM fleet_nodes n JOIN fleet_node_tokens t ON t.used_by=n.id WHERE t.id=${a.token_id} AND n.lifecycle<>'retired'`,
        )
        if (!enrolled) await provider.bootstrap(host, bootstrap, id)
        const node = await one<{ id: string }>(
          this.db,
          sql`SELECT n.id FROM fleet_nodes n JOIN fleet_node_tokens t ON t.used_by=n.id WHERE t.id=${a.token_id} AND n.lifecycle<>'retired'`,
        )
        await this.db.execute(sql`UPDATE provider_acquisitions SET host=${jsonb(host)} WHERE id=${id}`)
        if (node)
          await this.db.execute(sql`UPDATE provider_acquisitions SET node_id=${node.id} WHERE id=${id}`)
      } catch (error) {
        const refused = error instanceof ProviderError && error.code === 'refused'
        await this.db.execute(
          sql`UPDATE provider_acquisitions SET state=${refused ? 'refused' : 'uncertain'},error=${refused ? 'Provider refused request' : 'Provider outcome uncertain; reconcile before retry'} WHERE id=${id}`,
        )
      }
    })
  }
  async cancel(id: string) {
    return this.boxes.exclusive(`provider:${id}`, async () => this.cancelLocked(await this.record(id)))
  }
  private async cancelLocked(a: Acquisition) {
    const provider = this.provider(a.provider)
    await this.db.transaction(async (t) => {
      const token = await one<{ used_by: string | null }>(
        t,
        sql`SELECT used_by FROM fleet_node_tokens WHERE id=${a.token_id} FOR UPDATE`,
      )
      if (a.node_id || token?.used_by)
        throw new ProviderError(
          'refused',
          'This host enrolled; release it through its node and data-safety checks',
        )
      await t.execute(sql`UPDATE fleet_node_tokens SET expires_at=now() WHERE id=${a.token_id}`)
      await t.execute(sql`UPDATE provider_acquisitions SET cancel_requested=true WHERE id=${a.id}`)
    })
    if (!provider.managed) {
      await this.db.execute(sql`UPDATE provider_acquisitions SET state='cancelled' WHERE id=${a.id}`)
      return { state: 'owned_host_retained' }
    }
    const host = a.host ?? (await provider.list(a.id)).find((h) => h.requestKey === a.id)
    if (!host && a.state !== 'new') {
      await this.db.execute(
        sql`UPDATE provider_acquisitions SET state='uncertain',error='Cancellation waits for host discovery' WHERE id=${a.id}`,
      )
      return { state: 'uncertain' }
    }
    if (host) await provider.release(host, `cancel:${a.id}`)
    await this.db.execute(sql`UPDATE provider_acquisitions SET state='released',error=NULL WHERE id=${a.id}`)
    await event(this.db, 'provider.cancelled', { data: { id: a.id, provider: a.provider, host: host?.id } })
    return { state: 'released' }
  }
  async reconcile() {
    const pending = await rows<{ id: string }>(
      this.db,
      sql`SELECT id FROM provider_acquisitions WHERE state IN ('new','executing','uncertain','waiting') ORDER BY requested_at`,
    )
    for (const a of pending) this.queue(a.id)
    for (const provider of this.providers.values()) {
      let hosts: Host[] = []
      try {
        hosts = await provider.list('reconcile')
      } catch {
        continue
      }
      for (const host of hosts)
        if (
          !(await one(this.db, sql`SELECT 1 FROM provider_acquisitions WHERE id::text=${host.requestKey}`))
        ) {
          const first = await one(
            this.db,
            sql`INSERT INTO provider_orphans(provider,host) VALUES(${provider.name},${host.id}) ON CONFLICT DO NOTHING RETURNING host`,
          )
          if (first)
            await event(this.db, 'provider.orphan_found', {
              data: { provider: provider.name, host: host.id },
            })
        }
    }
  }
}
