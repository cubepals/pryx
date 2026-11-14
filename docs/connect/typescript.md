---
title: Connect a TypeScript backend
---

1. Install `@cubepals/pryx` in your Node backend with its package manager.
2. Set `PRYX_API_URL` and `PRYX_API_KEY` in the backend's private environment.
3. Run this module with your application's normal TypeScript runner or build.

```ts
import { Pryx } from '@cubepals/pryx'
const { PRYX_API_URL: url, PRYX_API_KEY: key } = process.env
if (!url || !key) throw new Error('Set PRYX_API_URL and PRYX_API_KEY')
const pryx = new Pryx({ url, key })
const defaults = (await pryx.project()).defaults
if (!defaults) throw new Error('The install did not report its default tier')
const box = await pryx.sandbox({
  isolation: defaults.isolation,
  ...(defaults.isolation === 'container' ? { image: 'alpine:3.22' } : {}),
})
try {
  const result = await box.run('echo hello')
  console.log(result.stdout)
  if (result.code !== 0) throw new Error('Command failed')
} finally { await box.delete() }
```

Keep this on the server: browser bundles must never contain a project key. On serverless
platforms, await work inside the request lifetime or use a supported background worker;
an unawaited promise may be terminated. Long streams need a runtime that permits their duration.
The host tier still controls [capabilities and isolation](../tiers.md).
[SDK reference](../../sdk/typescript/README.md).
