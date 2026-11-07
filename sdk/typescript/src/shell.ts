// SPDX-License-Identifier: FSL-1.1-ALv2
/** A private persistent pipe-shell handle; the agent serializes commands and owns framing. */
import type { Agent } from './agent.ts'
import { type Duration, seconds } from './duration.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import type { RunResult } from './run.ts'
export class Shell {
  constructor(
    private agent: Agent,
    readonly id: string,
  ) {}
  run(command: string, options: { timeout?: Duration } = {}) {
    return this.agent.requestV2<RunResult>(`/_pryx/shells/${encodeURIComponent(this.id)}/run`, 'POST', {
      command,
      timeout: seconds(options.timeout ?? SDK_DEFAULTS.runTimeoutSeconds),
    })
  }
  close() {
    return this.agent.requestV2<void>(`/_pryx/shells/${encodeURIComponent(this.id)}`, 'DELETE')
  }
}
