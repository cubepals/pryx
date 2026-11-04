// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A RivetKit actor owns session memory and one sandbox; credentials live in the server closure, never actor state. */
import type { ExecRequest, MakeRequest, Pryx } from '@cubepals/pryx'
import { actor } from 'rivetkit'

export interface SessionMemory {
  sandbox: string | null
  pending: { key: string; body: MakeRequest } | null
  generation: number
  notes: string[]
}
export function sandboxActor(client: Pryx, options: { sleepGracePeriod?: number } = {}) {
  return actor({
    state: { sandbox: null, pending: null, generation: 0, notes: [] } as SessionMemory,
    options,
    onWake: async (c) => {
      if (c.state.sandbox) await client.action(c.state.sandbox, 'wake')
    },
    onSleep: async (c) => {
      if (c.state.sandbox) await client.action(c.state.sandbox, 'sleep')
    },
    onDestroy: async (c) => {
      if (c.state.sandbox) await client.delete(c.state.sandbox)
      // A create reply can be lost before state receives its id. Recover the same keyed identity.
      else if (c.state.pending) {
        const made = await client.make(c.state.pending.body, c.state.pending.key)
        await client.delete(made.id)
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
        if (c.state.sandbox) return client.get(c.state.sandbox)
        if (Object.keys(body.secrets ?? {}).length)
          throw new Error(
            'Actor make does not persist secret-bearing requests; provision through a server-owned template',
          )
        if (!c.state.pending) {
          c.state.pending = { key: `rivet:${c.id}:${c.state.generation}`, body }
          await c.saveState({ immediate: true })
        }
        const made = await client.make(c.state.pending.body, c.state.pending.key)
        c.state.sandbox = made.id
        c.state.pending = null
        await c.saveState({ immediate: true })
        return made
      },
      exec: async (c, request: ExecRequest, key: string) => {
        if (!c.state.sandbox) throw new Error('Make a sandbox first')
        if (!key) throw new Error('Exec needs a durable caller key')
        return Array.fromAsync(client.exec(c.state.sandbox, request, key))
      },
      snapshot: async (c) => {
        if (!c.state.sandbox) throw new Error('Make a sandbox first')
        return client.snapshot(c.state.sandbox)
      },
      fork: async (
        _c,
        snapshot: string,
        body: Omit<MakeRequest, 'image' | 'snapshot' | 'template'>,
        key: string,
      ) => client.fork(snapshot, body, key),
      sleep: async (c) => {
        if (c.state.sandbox) await client.action(c.state.sandbox, 'sleep')
        c.sleep()
      },
      wake: async (c) => (c.state.sandbox ? client.action(c.state.sandbox, 'wake') : null),
      delete: async (c) => {
        if (c.state.sandbox) await client.delete(c.state.sandbox)
        else if (c.state.pending)
          await client.delete((await client.make(c.state.pending.body, c.state.pending.key)).id)
        c.state.sandbox = null
        c.state.pending = null
        c.state.generation++
        await c.saveState({ immediate: true })
      },
    },
  })
}
