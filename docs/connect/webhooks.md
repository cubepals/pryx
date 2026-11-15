---
title: Connect webhooks
---

1. Prepare your HTTPS receiver and choose a new file in an existing private directory for its signing secret.
2. Set `PRYX_API_URL`, `PRYX_API_KEY`, `PRYX_WEBHOOK_URL` and `PRYX_WEBHOOK_FILE` privately;
   install `@cubepals/pryx` in the Node program that registers the receiver.
3. Run this once. Keep the saved file private: the signing secret is returned only at registration.

```ts
import { writeFileSync } from 'node:fs'
import { Pryx } from '@cubepals/pryx'
const { PRYX_API_URL: url, PRYX_API_KEY: key, PRYX_WEBHOOK_URL: receiver, PRYX_WEBHOOK_FILE: file } = process.env
if (!url || !key || !receiver || !file) throw new Error('Set the four webhook environment variables')
const pryx = new Pryx({ url, key })
const hook = await pryx.webhook({ url: receiver, kinds: ['created', 'deleted'] })
try {
  writeFileSync(file, JSON.stringify({ id: hook.id, secret: hook.secret }) + '\n', { mode: 0o600, flag: 'wx' })
} catch (error) {
  await hook.delete()
  throw error
}
console.log('Webhook registered')
```

The file is created without overwriting an existing path; the block never prints the secret.
Verify timestamp and HMAC on the exact received bytes, deduplicate delivery IDs and acknowledge
promptly. [Signing, retries and dead letters](../api.md#lifecycle-webhooks) describe the receiver contract.
Private destinations need the host's allowlist; a caller cannot relax it. Events still reflect
the selected [tier's lifecycle limits](../tiers.md).
