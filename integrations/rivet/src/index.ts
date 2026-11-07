// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A RivetKit actor owns session memory and one sandbox; credentials live in the server closure, never actor state. */
import type { ExecRequest, MakeRequest, Pryx } from '@cubepals/pryx'
import { StatusCodes } from 'http-status-codes'
import { actor } from 'rivetkit'

export interface SessionMemory {
  sandbox: string | null
  pending: { key: string; body: MakeRequest } | null
  generation: number
  notes: string[]
}
export function sandboxActor(client: Pryx, options: { sleepGracePeriod?: number } = {}) {
  async function remove(id: string) {
    try {
      await (await client.sandbox(id)).delete()
    } catch (error) {
      if (!(error instanceof Error && 'status' in error && error.status === StatusCodes.GONE)) throw error
    }
  }
  return actor({
    state: { sandbox: null, pending: null, generation: 0, notes: [] } as SessionMemory,
    options,
    onWake: async (c) => {
      if (c.state.sandbox) await (await client.sandbox(c.state.sandbox)).wake()
    },
    onSleep: async (c) => {
      if (c.state.sandbox) await (await client.sandbox(c.state.sandbox)).sleep()
    },
    onDestroy: async (c) => {
      if (c.state.sandbox) await remove(c.state.sandbox)
      // A create reply can be lost before state receives its id. Recover the same keyed identity.
      else if (c.state.pending) {
        const made = await client.sandbox({ ...c.state.pending.body, idempotencyKey: c.state.pending.key })
        await made.delete()
      }
    },
    actions: {
      destroyActor: (c) => {
        c.destroy()
      },
      memory: (c) => c.state,
      remember: (c, note: string) => {
        c.state.notes.push(note)
      },
      make: async (c, body: MakeRequest) => {
        if (c.state.sandbox) return (await client.sandbox(c.state.sandbox)).toJSON()
        if (Object.keys(body.secrets ?? {}).length)
          throw new Error(
            'Actor make does not persist secret-bearing requests; provision through a server-owned template',
          )
        if (!c.state.pending) {
          c.state.pending = { key: `rivet:${c.id}:${c.state.generation}`, body }
          await c.saveState({ immediate: true })
        }
        const made = await client.sandbox({ ...c.state.pending.body, idempotencyKey: c.state.pending.key })
        c.state.sandbox = made.id
        c.state.pending = null
        await c.saveState({ immediate: true })
        return made.toJSON()
      },
      exec: async (c, request: ExecRequest, key: string) => {
        if (!c.state.sandbox) throw new Error('Make a sandbox first')
        if (!key) throw new Error('Exec needs a durable caller key')
        const box = await client.sandbox(c.state.sandbox)
        const run = box.stream(request.command, {
          env: request.env,
          timeout: request.timeout,
          idempotencyKey: key,
        })
        const events: unknown[] = []
        for await (const chunk of run) events.push({ type: chunk.stream, data: chunk.data, truncated: false })
        events.push({ type: 'exit', ...(await run.result()) })
        return events
      },
      snapshot: async (c) => {
        if (!c.state.sandbox) throw new Error('Make a sandbox first')
        return (await (await client.sandbox(c.state.sandbox)).snapshot()).view
      },
      fork: async (
        _c,
        snapshot: string,
        body: Omit<MakeRequest, 'image' | 'snapshot' | 'template'>,
        key: string,
      ) => client.sandbox({ ...body, snapshot, idempotencyKey: key }).then((box) => box.toJSON()),
      sleep: async (c) => {
        if (c.state.sandbox) await (await client.sandbox(c.state.sandbox)).sleep()
        c.sleep()
      },
      wake: async (c) =>
        c.state.sandbox
          ? (await client.sandbox(c.state.sandbox)).wake().then((box) => ({ sandbox: box.toJSON() }))
          : null,
      delete: async (c) => {
        if (c.state.sandbox) await remove(c.state.sandbox)
        else if (c.state.pending)
          await (
            await client.sandbox({ ...c.state.pending.body, idempotencyKey: c.state.pending.key })
          ).delete()
        c.state.sandbox = null
        c.state.pending = null
        c.state.generation++
        await c.saveState({ immediate: true })
      },
    },
  })
}
