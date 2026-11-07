// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Verify the composed node endpoint and direct Bun operator listener share native web responses. */

import { describe, expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { FleetCa } from '../src/ca.ts'
import { loadConfig } from '../src/config.ts'
import { startControl } from '../src/main.ts'
import { freshDatabase, hasDatabase } from './nodes.ts'

describe.skipIf(!hasDatabase)('control composition', () => {
  test('the operator listener still returns JSON after the node endpoint starts', async () => {
    const fresh = await freshDatabase()
    const pems = await FleetCa.generate('composition')
    const token = 'operator-token-for-the-composition-test'
    const config = loadConfig({
      DATABASE_URL: fresh.url,
      PRYX_DEPLOYMENT: 'composition',
      PRYX_CA_CERT: pems.certPem,
      PRYX_CA_KEY: pems.keyPem,
      PRYX_OPERATOR_TOKEN: token,
      S3_ENDPOINT: 'http://127.0.0.1:9000',
      S3_BUCKET: 'test',
      S3_ACCESS_KEY: 'test',
      S3_SECRET_KEY: 'test',
    })
    try {
      const control = await startControl({
        ...config,
        nodeListen: '127.0.0.1:0',
        operatorListen: '127.0.0.1:0',
        apiListen: '127.0.0.1:0',
      })
      try {
        const response = await fetch(`${control.operatorUrl}fleet/v1/nodes`, {
          headers: { authorization: `Bearer ${token}` },
        })
        expect(response.status).toBe(StatusCodes.OK)
        expect(await response.json()).toEqual({ nodes: [], regions: [] })
      } finally {
        await control.stop()
      }
    } finally {
      await fresh.drop()
    }
  })
})
