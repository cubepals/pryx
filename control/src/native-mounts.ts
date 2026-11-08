// SPDX-License-Identifier: FSL-1.1-ALv2
/** Mount credentials are sealed at the control storage owner and delivered only over the scoped
 * agent transport. Neither sandbox specs, operation records nor mount listings contain them. */
import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { z } from 'zod'
import type { sandboxTokenSchema } from './agents.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { agentSchema } from './node-contract.ts'
import { SandboxError } from './sandboxes.ts'
import { one, rows } from './sql.ts'

export const mountSchema = agentSchema('MountSpec')
export type MountSpec =
  import('../../sdk/typescript/src/generated/agent.ts').components['schemas']['MountSpec']
export class NativeMounts {
  constructor(
    private db: Db,
    private sealKey: string,
  ) {}
  private key() {
    if (this.sealKey.length < 32)
      throw new SandboxError('unsupported', 'Mount credential sealing is not configured')
    return createHash('sha256').update('pryx-mounts-v1\0').update(this.sealKey).digest()
  }
  seal(id: string, project: string, specs: MountSpec[]) {
    // AES-GCM uses the NIST-recommended 96-bit nonce and a full 128-bit authentication tag.
    const nonce = randomBytes(12),
      cipher = createCipheriv('aes-256-gcm', this.key(), nonce)
    cipher.setAAD(Buffer.from(JSON.stringify([id, project])))
    const ciphertext = Buffer.concat([cipher.update(JSON.stringify(specs)), cipher.final()])
    return Buffer.concat([nonce, cipher.getAuthTag(), ciphertext]).toString('base64')
  }
  open(id: string, project: string, sealed: string): MountSpec[] {
    const data = Buffer.from(sealed, 'base64'),
      decipher = createDecipheriv('aes-256-gcm', this.key(), data.subarray(0, 12))
    decipher.setAAD(Buffer.from(JSON.stringify([id, project])))
    decipher.setAuthTag(data.subarray(12, 28))
    return JSON.parse(
      Buffer.concat([decipher.update(data.subarray(28)), decipher.final()]).toString(),
    ) as MountSpec[]
  }
  async save(id: string, project: string, specs: MountSpec[]) {
    const sealed = this.seal(id, project, specs)
    await this.db.execute(
      sql`INSERT INTO sandbox_mounts(sandbox,project,sealed) VALUES(${id},${project},${sealed}) ON CONFLICT(sandbox) DO UPDATE SET sealed=excluded.sealed WHERE sandbox_mounts.project=excluded.project`,
    )
  }
  async remove(id: string) {
    await this.db.execute(sql`DELETE FROM sandbox_mounts WHERE sandbox=${id}`)
  }
  async initialize(id: string, project: string, credential: z.infer<typeof sandboxTokenSchema>) {
    const row = await one<{ sealed: string }>(
      this.db,
      sql`SELECT sealed FROM sandbox_mounts WHERE sandbox=${id} AND project=${project}`,
    )
    if (!row) return
    const mounts = this.open(id, project, row.sealed)
    const response = await fetch(`${credential.url}/_pryx/mounts`, {
      method: 'POST',
      headers: { authorization: `Bearer ${credential.token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ mounts }),
      redirect: 'manual',
      signal: AbortSignal.timeout(45 * SECOND_MS),
    })
    await response.body?.cancel()
    if (!response.ok)
      throw new SandboxError(
        'mount_failed',
        'The container tier refused an initial FUSE mount; check the endpoint and host native-tools policy',
      )
  }
  async maintain(run: (id: string, project: string) => Promise<unknown>) {
    const pending = await rows<{ id: string; project: string }>(
      this.db,
      sql`SELECT b.id,b.project FROM sandboxes b JOIN sandbox_mounts m ON m.sandbox=b.id WHERE b.deleted_at IS NULL AND b.state='running'`,
    )
    await Promise.all(
      Array.from({ length: Math.min(4, pending.length) }, async () => {
        while (pending.length) {
          const box = pending.shift()
          if (!box) break
          try {
            await run(box.id, box.project)
          } catch (error) {
            console.error(
              'Mount bootstrap failed',
              box.id,
              error instanceof SandboxError ? error.code : 'unavailable',
            )
          }
        }
      }),
    )
  }
}
