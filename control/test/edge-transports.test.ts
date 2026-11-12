// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real edge advertisements select HTTP SSH only with live edge and agent support. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { SECOND_MS } from '../src/durations.ts'
import { HTTP_ONLY_FEATURE, SSH_WEBSOCKET_FEATURE } from '../src/edge-headers.ts'
import { EDGE_SEEN_MS, sawEdge } from '../src/edge-routed.ts'
import { sshTransport } from '../src/edge-transports.ts'
import { publicContext } from './public-context.ts'

test('HTTP SSH requires advertised support and keeps live raw fronts default', async () => {
  const c = await publicContext()
  try {
    expect(await sshTransport(c.db, true)).toBeUndefined()
    await sawEdge(c.db, 'http-only', null, ['agent', SSH_WEBSOCKET_FEATURE, HTTP_ONLY_FEATURE])
    expect(await sshTransport(c.db, false)).toBeUndefined()
    expect(await sshTransport(c.db, true)).toBe('websocket')
    await sawEdge(c.db, 'legacy-raw', null)
    expect(await sshTransport(c.db, true)).toBe('tcp')
    await c.db.execute(
      sql`UPDATE edges SET seen_at=clock_timestamp()-((${EDGE_SEEN_MS / SECOND_MS}+1)*interval '1 second') WHERE id='legacy-raw'`,
    )
    expect(await sshTransport(c.db, true)).toBe('websocket')
    await sawEdge(c.db, 'raw', null, ['agent', SSH_WEBSOCKET_FEATURE])
    expect(await sshTransport(c.db, true)).toBe('tcp')
  } finally {
    await c.close()
  }
})
