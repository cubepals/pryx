---
title: Connect a Rivet harness
---

1. Install `@cubepals/pryx` in your Rivet runner project.
2. Set `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Replace the harness's sandbox factory with this export; keep the returned ID in actor state.

```ts
import { pryxProvider } from '@cubepals/pryx/provider'
export const sandboxProvider = pryxProvider()
```

The provider reads the default tier and uses an image only for containers. File paths resolve
inside the sandbox's reported `cwd`. Shared-UID processes must be mutually trusted; internet-enabled
processes share host networking. Wasm has a virtual kernel and shared sidecar, with HTTP-only ingress.
Named missing tiers retain admission refusals; this API version refuses named guarantees it cannot prove.

[The full integration guide](../integrations/rivet.md) covers lifecycle semantics and partial methods.
OpenOrange 1.0.2 needs the prepared upstream patch; it has not accepted or released that provider hook.
Follow that guide before trying its registration or preview callback.
