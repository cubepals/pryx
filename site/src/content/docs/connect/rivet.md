---
title: Connect Rivet and OpenOrange
---

Use your pryx install as the sandbox for a Rivet agent harness. The SDK's provider follows
Rivet's adapter contract; the Actor keeps its sandbox id and existing lifecycle hooks.

## Rivet agent harness

1. Install `@cubepals/pryx` in the runner project.
2. Set `PRYX_API_URL` and `PRYX_API_KEY` to your install's address and project key.
3. Replace the harness's sandbox factory with this export.

```ts
import { pryxProvider } from '@cubepals/pryx/provider'
export const sandboxProvider = pryxProvider()
```

## OpenOrange project

OpenOrange 1.0.2 does not yet accept a custom provider. The prepared patch in
`docs/upstream/open-orange/` adds the named `pryx` provider; it has not been sent or released.
These three steps describe the registration once that change is accepted:

1. Install `@cubepals/pryx` next to OpenOrange.
2. Set `PRYX_API_URL`, `PRYX_API_KEY` and `ORANGE_SANDBOX_PROVIDER=pryx` on the runner.
3. Register the provider and preview callback below; keep your Sandbox Worker code unchanged.

```ts
import { pryxProvider, pryxPortAddress } from '@cubepals/pryx/provider'
export const sandboxProvider = pryxProvider()
export const portAddress = (id: string, port: number) => pryxPortAddress(id, port)
```

The default working directory is persistent `/data`; native file paths must stay inside it.
The host chooses the tier. You may name `tier`, `template` and `require`; unavailable tiers
retain the install's refusal, and this API version refuses every named guarantee because
it cannot prove it. Sleep keeps files and connect wakes the sandbox. Previews go through
pryx's edge. The expiring pryx browser capability does not satisfy OpenOrange's fixed CDP
endpoint setting. See the repository's `docs/integrations/rivet.md` for execution semantics,
versioned interfaces and the binding's remaining partial methods.
