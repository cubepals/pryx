// SPDX-License-Identifier: FSL-1.1-ALv2
/** Render paired shell evidence without inventing a session latency for the baseline. */
import { appendFileSync, readFileSync } from 'node:fs'

type Report = {
  source: string
  fresh: { n: number; p50Ms: number; p90Ms: number }
  persistentShell: { n: number; p50Ms?: number; p90Ms?: number; mechanism?: string }
}
const [beforePath, afterPath] = process.argv.slice(2)
if (!beforePath || !afterPath) throw new Error('field-summary.ts before.json after.json')
const before = JSON.parse(readFileSync(beforePath, 'utf8')) as Report,
  after = JSON.parse(readFileSync(afterPath, 'utf8')) as Report
const format = (value?: number) => (value === undefined ? 'Unavailable' : value.toFixed(2))
const row = (name: string, data: { n: number; p50Ms?: number; p90Ms?: number }) =>
  ['| ' + name, data.n, format(data.p50Ms), format(data.p90Ms) + ' |'].join(' | ')
const text = [
  '## K16 shell measurements',
  '',
  '| Path | n | p50 ms | p90 ms |',
  '| --- | ---: | ---: | ---: |',
  row('K15 fresh process', before.fresh),
  row('K16 fresh process', after.fresh),
  row('K16 persistent shell', after.persistentShell),
  '',
  'Baseline ' +
    before.source +
    '; current ' +
    after.source +
    '. ' +
    after.persistentShell.mechanism +
    '. Each sample verifies output and status; the session also verifies cwd, environment and separate stderr. Raw samples are in the measurement artifact.',
  '',
].join('\n')
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, text)
console.log(text)
