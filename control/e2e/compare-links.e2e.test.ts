// SPDX-License-Identifier: FSL-1.1-ALv2
/** The comparison is evidence, not an unverified feature list. The existing E2E suite wraps
 * curl for live citation checks and filesystem stat for local implementation links. */
import { expect, test } from 'bun:test'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { MINUTE_MS } from '../src/durations.ts'

test(
  'every comparison citation and implementation link resolves',
  async () => {
    const document = readFileSync('docs/compare.md', 'utf8')
    const links = [...new Set([...document.matchAll(/\]\(([^)]+)\)/g)].map((m) => m[1] as string))]
    const results: { url: string; checkedAt: string; ok: boolean }[] = []
    const pending = [...links]
    await Promise.all(
      Array.from({ length: 4 }, async () => {
        while (pending.length) {
          const url = pending.shift()
          if (!url) break
          let ok: boolean
          if (url.startsWith('https://')) {
            const child = Bun.spawn(
              [
                '/usr/bin/curl',
                '--fail',
                '--silent',
                '--show-error',
                '--location',
                '--max-time',
                '30',
                '--retry',
                '3',
                '--retry-delay',
                '1',
                '--output',
                '/dev/null',
                url,
              ],
              { stdout: 'ignore', stderr: 'pipe' },
            )
            await new Response(child.stderr).text()
            ok = (await child.exited) === 0
          } else ok = existsSync(resolve('docs', url.split('#')[0] ?? ''))
          results.push({ url, checkedAt: new Date().toISOString(), ok })
        }
      }),
    )
    writeFileSync(
      process.env.PRYX_COMPARE_LINKS ?? '/tmp/pryx-compare-links.json',
      `${JSON.stringify(results, null, 2)}\n`,
    )
    expect(results.filter((r) => !r.ok).map((r) => r.url)).toEqual([])
    for (const line of document
      .split('\n')
      .filter(
        (line) => line.startsWith('| ') && !line.startsWith('| ---') && !line.startsWith('| Capability'),
      )) {
      const cells = line.split('|').slice(2, -1)
      expect(cells).toHaveLength(4)
      expect(cells.every((cell) => /\]\([^)]+\)/.test(cell))).toBe(true)
      expect(cells.slice(1).every((cell) => cell.includes('2026-11-08'))).toBe(true)
    }
  },
  8 * MINUTE_MS,
)
