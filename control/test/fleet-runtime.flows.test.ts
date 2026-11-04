// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet flow regression suites over Postgres and node doubles. */

import { describe } from 'bun:test'
import { registerFlows1 } from './flow-checks-1.ts'
import { registerFlows2 } from './flow-checks-2.ts'
import { flowContext } from './flow-context.ts'
import { hasDatabase } from './nodes.ts'

describe.skipIf(!hasDatabase)('FleetRuntime flows, with pryxd in memory', () => {
  const ctx = flowContext()
  registerFlows1(ctx)
  registerFlows2(ctx)
})
