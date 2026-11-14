# Rivet and OpenOrange

The TypeScript SDK exports `pryxProvider` from `@cubepals/pryx/provider`. It implements
Rivet's published `SandboxProvider` contract. Install the SDK alongside your existing adapter
and set `PRYX_API_URL` and `PRYX_API_KEY` in the actor runner's environment. Keep the key there;
it never belongs in a Worker response or sandbox environment.

## Agent harness in a Rivet Actor

1. Install `@cubepals/pryx` alongside your harness.
2. Set the install's `PRYX_API_URL` and `PRYX_API_KEY` on the runner.
3. Use this factory for the harness's sandbox provider. Keep its existing create/connect,
   sleep and destroy hooks; the Actor stores the sandbox id as before.

```ts
import { pryxProvider } from '@cubepals/pryx/provider'
export const sandboxProvider = pryxProvider()
```

Run the complete lifecycle example with `bun examples/rivet-provider.ts`. It needs no model
key. The provider holds no per-Actor state. `create` returns an id, `connect` starts stopped compute or wakes a sleeping
sandbox, `suspend` sleeps it with data kept, and `destroy` deletes it. Connecting to a deleted
or expired sandbox returns `undefined`; authentication, placement and transport failures throw.
A missing sandbox makes suspend/destroy a no-op. Mutations are sent once.

## OpenOrange Sandbox binding

OpenOrange 1.0.2 has a closed list of provider names and no custom-provider registration API.
It cannot select pryx in an unmodified release. The [prepared patch](../upstream/open-orange/README.md)
adds that registration; it is a proposal, has not been sent upstream, and is not part of the
published OpenOrange package. After that change is accepted, the connection is:

1. Install `@cubepals/pryx` next to OpenOrange.
2. Set `PRYX_API_URL`, `PRYX_API_KEY` and `ORANGE_SANDBOX_PROVIDER=pryx` on the runner.
3. The provider registration below backs the existing Sandbox binding; Worker code keeps
   its existing `getSandbox()` calls. The prepared patch supplies the native file/preview bridge.

```ts
import { pryxProvider, pryxPortAddress } from '@cubepals/pryx/provider'
export const sandboxProvider = pryxProvider()
export const portAddress = (id: string, port: number) => pryxPortAddress(id, port)
```

The binding uses the connected sandbox's persistent directory: `/data` on mounted tiers,
or the agent's data directory on the process tier. Native file paths must stay inside that root.
The host's default tier applies; a caller can pass
`{ tier, template, require }` to the factory, along with optional `url` and `key` overrides.
The current API predates K19's `require`/`guarantees` contract: every nonempty `require` is
refused with `missing_guarantee` and the names, before creation. No requirement is silently dropped.
Unavailable tiers and templates retain the API's reason. Container creation uses the project's
configured `repoImage` when no template was named. A template used with this provider must
mount its persistent storage at `/data` on mounted tiers. Use `sandbox.cwd` to resolve file
paths and execution directories; the provider learns the process directory when connecting
and leaves the optional provider-level `cwd` unset.

Exec accepts absolute `cwd`, an explicit environment, a millisecond timeout and an AbortSignal.
Undefined or zero timeout adds no provider deadline; the install's execution ceiling still
applies. Timeout kills the process group and returns `timedOut: true` and `exitCode: null`.
Abort kills it and rejects with `Error('aborted')`, including an abort during process startup.
`onData` receives stdout and stderr as they arrive; final output keeps the streams separate.
Nonzero exits are results. Output above the SDK's capture budget throws `output_truncated`;
connection loss keeps the SDK's unknown-outcome error and never replays the command.

`pryxPortAddress(id, port)` publishes a port through pryx's own edge and returns its preview
URL. No sandbox IP or direct host listener is returned. Capacity and host policy refusals
remain visible. The prepared binding patch sends preview requests through that address.

OpenOrange's other partial and unsupported methods stay as documented by its
[Sandbox binding](https://raw.githubusercontent.com/rivet-dev/open-orange/1cabf2e9db300361c9f0b751c7723da7eb706f5e/docs/modules/sandbox.mdx):
its session/code-context methods throw, inactivity timeout does nothing, container images and
custom entrypoints are not run, and PTY/stdin, interception, container snapshots and inspection
are unsupported. Full `Files.stat/lstat` metadata remains unavailable through this bridge.

## Browser Rendering

The current pryx browser is **not a fixed OpenOrange Browser Rendering endpoint**.
`box.browser()` mints a capability URL that expires within five minutes; minting another
rotates access, and sleep/recreation also invalidates old access. OpenOrange reads one
`ORANGE_BROWSER_CDP_URL`, opening a new connection to it for each launch, and has no callback
for refreshing that capability. Do not place a minted pryx browser URL in that variable.
No persistent browser compatibility is claimed. The [Browser Rendering rules](https://raw.githubusercontent.com/rivet-dev/open-orange/1cabf2e9db300361c9f0b751c7723da7eb706f5e/docs/modules/browser-rendering.mdx)
require a URL that accepts each new connection; pryx's short-lived browser access remains
available through its own SDK.

## Recorded interfaces

Read on 2026-11-14: sandbox-adapter **0.5.6**, Apache-2.0, from the integrity-pinned npm package;
OpenOrange **1.0.2**, Apache-2.0, commit `1cabf2e9db300361c9f0b751c7723da7eb706f5e`.
The published adapter declares abort as `Error('aborted')`, timeout as a null exit and a flag,
and missing connections as undefined. Its Docker provider stops/restarts on suspend/connect;
its E2B provider pauses and resumes; normal command failures remain results. The declarations
are a development dependency only. CI typechecks this version on every change and the newest
published version on its existing weekly schedule. Real pryx acceptance exercises every provider
method, cancellation, streaming, sleep/wake persistence, missing connections and edge preview.
Both pages' blocks and the complete example run in that same deployment job.
