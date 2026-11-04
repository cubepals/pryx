# TypeScript SDK

`@cubepals/pryx` is the public v1 client. Types are generated with the pinned
openapi-typescript from control/openapi/pryx.openapi.json. `bun run openapi:generate`
updates them and `bun run openapi:check` checks drift.

```ts
import { Pryx } from '@cubepals/pryx'
const pryx = new Pryx({ baseUrl: process.env.PRYX_API_URL!, apiKey: process.env.PRYX_API_KEY! })
const sandbox = await pryx.make({ image: 'alpine:3.22', entrypoint: ['sh', '-c', 'while :; do sleep 1; done'] }, crypto.randomUUID())
try {
  for await (const event of pryx.exec(sandbox.id, { command: ['sh', '-c', 'echo hello > /data/message; cat /data/message'] }, crypto.randomUUID())) console.log(event)
  const snapshot = await pryx.snapshot(sandbox.id)
  const fork = await pryx.fork(snapshot.id)
  try { console.log(await pryx.get(fork.id)) } finally { await pryx.delete(fork.id) }
} finally { await pryx.delete(sandbox.id) }
```

Supply a stable Idempotency-Key when retrying create or exec. An uncertain exec returns
`operation_unknown` on retry instead of risking a second command. The client sends no automatic
mutation retries. Exec uses NDJSON stdout/stderr/exit events emitted after the node's buffered
command completes, preserving truncation, timeout and killed fields. Logs stream as they arrive;
pass an AbortSignal for follow mode. Per-command env requires `env` inside the image.

File paths are relative to persistent storage. `putFile` takes base64 (at most 48 KiB decoded);
`getFile` returns a base64 tar archive. The image needs sh, realpath, base64 and tar. Oversized
or failed reads are refused. Errors throw PryxError with status, code and details.

Release archives include a bundled ESM JavaScript entry for Node and the generated TypeScript
source/types. Bun workspace imports use source. `bun run release:build` builds the Node entry
before `bun pm pack`; the SDK version is checked against Cargo.toml.
