<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Interfaces checked before implementation

Rivet and rivet-dev/actors currently resolve to the same repository and commit,
ac941869c79074cee5324f63d5057088a6581a2b. We read the Actors lifecycle, agentOS and Platform
sources/docs and the sandbox coding-agent example. Relevant interfaces:

- [RivetKit actor config/hooks](https://github.com/rivet-dev/rivet/blob/ac941869c79074cee5324f63d5057088a6581a2b/rivetkit-typescript/packages/rivetkit/src/actor/config.ts#L1597): onDestroy, onWake and onSleep; actions and persisted state are the supported actor surface.
- [Current agentOS entry point](https://github.com/rivet-dev/rivet/blob/ac941869c79074cee5324f63d5057088a6581a2b/rivetkit-typescript/packages/rivetkit/src/agent-os/index.ts): explicitly removed from RivetKit and moved to the separate agentos project. Old sandbox examples alone do not establish a current exported provider interface.
- [agentOS external sandbox contract](https://github.com/rivet-dev/agentos/blob/785e6441b7fb511857f28e3b2e11f2f655165d58/packages/core/src/sandbox.ts#L36): AgentOsSandboxClient and AgentOsSandboxProvider.start, covering real background processes, stdin, logs and execution. The published core package is pinned to 0.4.7.
- [agentOS's sandbox-agent bridge](https://github.com/rivet-dev/agentos/blob/785e6441b7fb511857f28e3b2e11f2f655165d58/packages/agentos-sandbox/src/provider.ts): uses SandboxAgent.start and maps disposal to destroySandbox. Our adapter follows this public boundary, with sandbox-agent 0.5.5's exported SandboxProvider type imported directly. Its published package owns the server image/CLI contract. This is an external sandbox backend, not an implementation of agentOS's VM runtime.

celld was read at 8702f6d186c77698dbfb30cf29e3b1922a88f008:

- [ContainerEngine and concrete discovery](https://github.com/denoland/celld/blob/8702f6d186c77698dbfb30cf29e3b1922a88f008/crates/celld/container.rs#L373): local Docker/Podman socket discovery, no backend trait/selector. start/exec/address/monitor effects live in that same concrete engine.
- [V8 container ops](https://github.com/denoland/celld/blob/8702f6d186c77698dbfb30cf29e3b1922a88f008/crates/celld/js/container.rs#L13): bind ctx.container directly to ContainerEngine, with local address and process semantics.
- [Ownership effects](https://github.com/denoland/celld/blob/8702f6d186c77698dbfb30cf29e3b1922a88f008/crates/celld/ownership_store.rs): conditional object writes delegate ownership decisions to celld-logic; record mismatch/missing state is a fence, never an unconditional overwrite.
- [Self-fencing guarantees](https://github.com/denoland/celld/blob/8702f6d186c77698dbfb30cf29e3b1922a88f008/docs/guarantees.md#self-fencing): renew every third of a term, stop cells and fail requests at expiry, terminal self-fence, ingress checks deadlines. Idle eviction retains container state; ownership loss destroys it, per ContainerEngine.Release.

The celld package therefore contains the permitted proposal and a Worker/DO transport prototype
against an unmodified release. It does not claim that celld already accepts a native pryx backend.
Store-lease implementation and fault-model/measurement limits are documented separately in
[store-lease](../docs/store-lease.md).
