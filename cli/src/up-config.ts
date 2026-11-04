// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Generate one-box credentials in root-only unit environments; retain them unchanged on later runs. */

import { randomBytes } from 'node:crypto'

import { join } from 'node:path'
import { FleetCa } from '../../control/src/ca.ts'
import { readEnvironment, writeEnvironment } from './up-files.ts'

const secret = () => randomBytes(32).toString('hex')
// AWS IAM credential format: 20-character access id and 40-character secret; S3-compatible stores accept these.
const S3_ACCESS_RANDOM_BYTES = 10
const S3_SECRET_RANDOM_BYTES = 20
export async function configurations(domain: string, folder = '/etc/pryx') {
  const control = readEnvironment(join(folder, 'control.env'))
  const fresh = !control.PRYX_CA_KEY
  if (fresh) {
    const ca = await FleetCa.generate('pryx'),
      password = secret(),
      access = randomBytes(S3_ACCESS_RANDOM_BYTES).toString('hex'),
      storeSecret = randomBytes(S3_SECRET_RANDOM_BYTES).toString('hex')
    Object.assign(control, {
      DATABASE_URL: `postgres://pryx:${password}@127.0.0.1:5432/pryx`,
      PRYX_DEPLOYMENT: 'pryx',
      PRYX_CA_CERT: ca.certPem.replaceAll('\n', '\\n'),
      PRYX_CA_KEY: ca.keyPem.replaceAll('\n', '\\n'),
      PRYX_OPERATOR_TOKEN: secret(),
      PRYX_EDGE_TOKEN: secret(),
      PRYX_BOOTSTRAP_API_KEY: `pryx_${randomBytes(32).toString('base64url')}`,
      PRYX_API_LISTEN: '127.0.0.1:8082',
      PRYX_OPERATOR_LISTEN: '127.0.0.1:8081',
      PRYX_NODE_LISTEN: '127.0.0.1:8443',
      PRYX_ENDPOINT_HOSTS: '127.0.0.1,localhost',
      PRYX_PRYXD_BIN: '/usr/local/bin/pryxd',
      PRYX_ASSET_DIR: '/opt/pryx/control',
      S3_ENDPOINT: 'http://127.0.0.1:9000',
      S3_BUCKET: 'pryx-snapshots',
      S3_ACCESS_KEY: access,
      S3_SECRET_KEY: storeSecret,
    })
    writeEnvironment(join(folder, 'postgres.env'), {
      POSTGRES_USER: 'pryx',
      POSTGRES_DB: 'pryx',
      POSTGRES_PASSWORD: password,
      POSTGRES_INITDB_ARGS: '--auth-host=scram-sha-256',
    })
    writeEnvironment(join(folder, 'store.env'), {
      RUSTFS_ACCESS_KEY: access,
      RUSTFS_SECRET_KEY: storeSecret,
      RUSTFS_CONSOLE_ENABLE: 'false',
    })
  }
  control.PRYX_PUBLIC_DOMAIN = domain
  const changed = writeEnvironment(join(folder, 'control.env'), control)
  const edgeChanged = writeEnvironment(join(folder, 'edge.env'), {
    PRYX_CONTROL_URL: 'http://127.0.0.1:8082',
    PRYX_EDGE_TOKEN: control.PRYX_EDGE_TOKEN as string,
    PRYX_EDGE_HOST: '0.0.0.0',
    PRYX_EDGE_METRICS_HOST: '127.0.0.1',
  })
  return { control, changed, edgeChanged, fresh }
}
