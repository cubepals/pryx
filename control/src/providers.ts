// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Provider composition only; scaling policy and durable request state live outside this directory.
 * Parts (`providers/`):
 * - `provider.ts`: host/shape/bootstrap contracts and sanitized errors.
 * - `client.ts`: one bounded HTTP attempt with no automatic mutation retry.
 * - `hetzner.ts`: server, network/firewall attachment and cloud-init.
 * - `boat.ts`: idempotent VM creation and checked private/systemd bootstrap.
 * - `manual.ts`: strict SSH on an explicitly owned host, never released.
 * - `fake.ts`: deterministic provider for tests.
 * - `private-network.ts`: the documented private IP classes.
 */
import { z } from 'zod'
import { BoatProvider } from './providers/boat.ts'
import { HetznerProvider } from './providers/hetzner.ts'
import { ManualProvider } from './providers/manual.ts'
import { type Provider, shapeSchema } from './providers/provider.ts'

export type { Bootstrap, Host, Provider, Shape } from './providers/provider.ts'
export function createProviders(env: Record<string, string | undefined>, deployment: string) {
  const providers = new Map<string, Provider>()
  const number = (name: string) => z.number().int().positive().parse(Number(env[name]))
  if (env.PRYX_HETZNER_TOKEN)
    providers.set(
      'hetzner',
      new HetznerProvider({
        token: env.PRYX_HETZNER_TOKEN,
        deployment,
        network: number('PRYX_HETZNER_NETWORK_ID'),
        firewall: number('PRYX_HETZNER_FIREWALL_ID'),
        sshKeys: JSON.parse(env.PRYX_HETZNER_SSH_KEYS ?? '[]') as string[],
        image: env.PRYX_HETZNER_IMAGE ?? 'ubuntu-24.04',
      }),
    )
  if (env.PRYX_BOAT_TOKEN)
    providers.set(
      'boat',
      new BoatProvider({
        token: env.PRYX_BOAT_TOKEN,
        deployment,
        region: env.PRYX_BOAT_REGION ?? 'boat',
        catalog: z.array(shapeSchema).parse(JSON.parse(env.PRYX_BOAT_SHAPES ?? '[]')),
        networkSetup: env.PRYX_BOAT_NETWORK_SETUP ?? '',
        privateAddressCommand: env.PRYX_BOAT_PRIVATE_ADDRESS_COMMAND ?? '',
      }),
    )
  if (env.PRYX_MANUAL_ADDRESS)
    providers.set(
      'manual',
      new ManualProvider({
        address: env.PRYX_MANUAL_ADDRESS,
        ...(env.PRYX_MANUAL_SSH_ADDRESS ? { sshAddress: env.PRYX_MANUAL_SSH_ADDRESS } : {}),
        region: env.PRYX_MANUAL_REGION ?? 'default',
        shape: shapeSchema.parse(JSON.parse(env.PRYX_MANUAL_SHAPE ?? '{}')),
        keyPath: env.PRYX_MANUAL_SSH_KEY,
        user: env.PRYX_MANUAL_SSH_USER,
        ...(env.PRYX_MANUAL_SSH_PORT ? { port: number('PRYX_MANUAL_SSH_PORT') } : {}),
      }),
    )
  return providers
}
