// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

import { Pryx } from '@cubepals/pryx'
import { setup } from 'rivetkit'
import { sandboxActor } from '../src/index.ts'

const baseUrl = process.env.PRYX_API_URL,
  apiKey = process.env.PRYX_API_KEY
if (!baseUrl || !apiKey) throw new Error('Set PRYX_API_URL and PRYX_API_KEY on the actor server')
export const registry = setup({
  use: { codingAgent: sandboxActor(new Pryx({ url: baseUrl, key: apiKey })) },
  startEngine: true,
  startServices: false,
  httpHost: '127.0.0.1',
  ...(process.env.RIVET_HTTP_PORT ? { httpPort: Number(process.env.RIVET_HTTP_PORT) } : {}),
  ...(process.env.RIVET_ENGINE_PORT ? { enginePort: Number(process.env.RIVET_ENGINE_PORT) } : {}),
})
if (import.meta.main) registry.start()
