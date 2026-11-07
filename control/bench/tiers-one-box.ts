// SPDX-License-Identifier: FSL-1.1-ALv2
/** Use the installed one-box storage with isolated cohort databases and buckets. No credential
 * is printed or included in measurement reports. This entrypoint requires its root-owned config. */
import { readEnvironment } from '../../cli/src/up-files.ts'

const config = readEnvironment('/etc/pryx/control.env')
const child = Bun.spawn(
  [process.execPath, 'control/bench/tiers.ts', 'one-box', process.argv[2] ?? 'tiers-one-box'],
  {
    env: {
      ...process.env,
      DATABASE_URL: config.DATABASE_URL,
      S3_TEST_ENDPOINT: config.S3_ENDPOINT,
      S3_TEST_ACCESS_KEY_ID: config.S3_ACCESS_KEY,
      S3_TEST_SECRET_ACCESS_KEY: config.S3_SECRET_KEY,
      PRYX_TIER_BENCH: '1',
      PRYXD_SUDO: '0',
    },
    stdio: ['inherit', 'inherit', 'inherit'],
  },
)
process.exit(await child.exited)
