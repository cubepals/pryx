// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Bootstrap an explicitly owned host over strict SSH. A manual host is never released or autoscaled away. */

import { isIP } from 'node:net'
import { MINUTE_MS, SECOND_MS } from '../durations.ts'

import { privateIp } from './private-network.ts'
import {
  type Bootstrap,
  bootstrapScript,
  type Host,
  type Provider,
  ProviderError,
  type Shape,
  shellQuote,
} from './provider.ts'
export class ManualProvider implements Provider {
  readonly name = 'manual'
  readonly managed = false
  constructor(
    readonly options: {
      address: string
      sshAddress?: string
      region: string
      shape: Shape
      keyPath?: string
      user?: string
      port?: number
      run?: (args: string[], input: string) => Promise<string>
    },
  ) {
    if (!privateIp(options.address))
      throw new ProviderError('refused', 'Manual address must name the private network')
    if (!isIP(options.address) && !/^[a-zA-Z0-9][a-zA-Z0-9.-]*$/.test(options.address))
      throw new Error('Manual address is an IP or hostname')
  }
  private async ssh(command: string, input = '', timeoutMs = 30 * SECOND_MS) {
    const args = [
      'ssh',
      '-o',
      'BatchMode=yes',
      '-o',
      'StrictHostKeyChecking=yes',
      ...(this.options.keyPath ? ['-i', this.options.keyPath] : []),
      ...(this.options.port ? ['-p', String(this.options.port)] : []),
      `${this.options.user ?? 'root'}@${this.options.sshAddress ?? this.options.address}`,
      command,
    ]
    if (this.options.run) return this.options.run(args, input)
    const child = Bun.spawn(args, { stdin: 'pipe', stdout: 'pipe', stderr: 'pipe', timeout: timeoutMs })
    child.stdin.write(input)
    child.stdin.end()
    const output = await new Response(child.stdout).text()
    await new Response(child.stderr).text()
    if ((await child.exited) !== 0) throw new ProviderError('uncertain', 'SSH bootstrap did not finish')
    return output
  }
  async acquire(region: string, shape: Shape, key: string, bootstrap: Bootstrap) {
    if (region !== this.options.region || shape.name !== this.options.shape.name)
      throw new ProviderError('refused', 'Manual host does not match this region/shape')
    const host = {
      id: this.options.address,
      region,
      address: this.options.address,
      requestKey: key,
      shape: shape.name,
    }
    await this.bootstrap(host, bootstrap, key)
    return host
  }
  async bootstrap(_host: Host, bootstrap: Bootstrap, key: string) {
    await this.ssh(
      'sudo install -d -m 0700 /var/lib/pryx-provider && sudo flock -x /var/lib/pryx-provider/bootstrap.lock sh -s',
      bootstrapScript(
        { ...bootstrap, line: `${bootstrap.line} --address ${shellQuote(this.options.address)}` },
        key,
      ),
      10 * MINUTE_MS,
    )
  }
  async release(_host: Host, _key: string) {} // Owned hosts are never returned to a vendor.
  async list() {
    const key = (
      await this.ssh(
        "sudo sh -c 'if [ -f /var/lib/pryx-provider/acquire-key ]; then cat /var/lib/pryx-provider/acquire-key; fi'",
      )
    ).trim()
    if (!key) return []
    return [
      {
        id: this.options.address,
        region: this.options.region,
        address: this.options.address,
        requestKey: key,
        shape: this.options.shape.name,
      },
    ]
  }
  async shapes(region: string) {
    return region === this.options.region ? [this.options.shape] : []
  }
}
