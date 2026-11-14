// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real container create-to-first-PNG cohorts match the native process timer. Existing warm
 * execution/restart budgets remain in interpreter.e2e.test.ts and are never replaced here. */
import { expect, test } from 'bun:test'
import { readFileSync, writeFileSync } from 'node:fs'
import { builtinTemplate } from '../../cli/src/builtin-template.ts'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'

test(
  'container interpreters retain thirty create-to-first-image samples per language',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('container')
    let box: Sandbox | undefined
    const samples: Record<string, number[]> = {}
    try {
      for (const [language, name, image] of [
        ['python', 'python-interpreter', 'alpine:pryx-python-interpreter'],
        ['javascript', 'node-interpreter', 'alpine:pryx-node-interpreter'],
      ] as const) {
        const plan = builtinTemplate(name, image)
        if (!plan) throw new Error('Missing interpreter template')
        await c.sdk.template(plan)
        // Prepare the template before starting the cohort, matching the cached process bundle.
        box = await c.sdk.sandbox({ template: name, isolation: 'container', units: 4, idle: '10m' })
        await c.route(box.id)
        const prepared = language === 'python' ? await box.python() : await box.javascript()
        expect((await prepared.run('1+1')).text).toBe('2')
        await box.delete()
        box = undefined
        const values: number[] = []
        for (let sample = 0; sample < 30; sample++) {
          const start = performance.now()
          box = await c.sdk.sandbox({ template: name, isolation: 'container', units: 4, idle: '10m' })
          await c.route(box.id)
          if (language === 'javascript')
            await box.write('chart.cjs', readFileSync('sdk/typescript/test/fixtures/chart.cjs'))
          const kernel = language === 'python' ? await box.python() : await box.javascript()
          const image = (
            await kernel.run(
              language === 'python'
                ? 'plt.plot([1,2,3]); plt.show()'
                : "$$.mime({'image/png':require('./chart.cjs')().toString('base64')})",
            )
          ).png
          expect(image).toBeDefined()
          expect([...(image?.subarray(0, 8) ?? [])]).toEqual([137, 80, 78, 71, 13, 10, 26, 10])
          expect(image?.length).toBeGreaterThan(100)
          values.push(performance.now() - start)
          await box.delete()
          box = undefined
        }
        expect(values).toHaveLength(30)
        samples[language] = values
      }
      const summary = Object.fromEntries(
        Object.entries(samples).map(([name, values]) => {
          const sorted = [...values].sort((a, b) => a - b)
          return [name, { n: values.length, p50Ms: sorted[14], p90Ms: sorted[26] }]
        }),
      )
      console.log('Container interpreter create-to-image measurements', JSON.stringify(summary))
      if (process.env.PRYX_INTERPRETER_IMAGE_MEASUREMENTS)
        writeFileSync(
          process.env.PRYX_INTERPRETER_IMAGE_MEASUREMENTS,
          JSON.stringify(
            {
              isolation: 'container',
              units: 4,
              point: 'create-to-first-image',
              unit: 'ms',
              samples,
              summary,
            },
            null,
            2,
          ),
        )
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box?.delete()
      await c.close()
    }
  },
  10 * MINUTE_MS,
)
