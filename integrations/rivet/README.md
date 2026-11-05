<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# pryx for RivetKit

This optional package depends on the public SDK and pinned RivetKit 3.0.1. Nothing in the daemon,
control or edge imports it. `sandboxActor(client)` keeps session notes and one sandbox id in
actor state, with make, exec, snapshot, fork, sleep, wake and delete actions. Its actual onSleep,
onWake and onDestroy hooks sleep, wake and delete that sandbox. A lost create reply is retried
with the actor's persisted key; exec requires a durable caller key. A fork returns a separate
sandbox for the caller to manage. Actor state is ordinary persisted session memory: do not put
credentials in notes or env. Make refuses secret-bearing requests; use a server-owned template.
Choose a sandbox TTL as a cleanup backstop for an actor host that crashes before destruction hooks.

Run the coding-agent example with PRYX_API_URL and PRYX_API_KEY on the server:

```sh
bun integrations/rivet/example/main.ts
```

A client connects with `createClient<typeof registry>()` from `rivetkit/client`, gets
`client.codingAgent.getOrCreate([sessionId])`, calls `remember` for its session memory, then
`make({image:'alpine:3.22', entrypoint:['sh','-c','while :; do sleep 1; done']})` and
`exec({command:['sh','-c','echo hello > /data/hello']}, commandId)` for its shell. Read `memory()`
and the exec result to feed a model of your choice; no model credential is needed for the example.
`sleep()` puts both actor and sandbox to sleep; the next actor call invokes the wake hook.
Configure Rivet authentication before exposing the actor host to untrusted clients.

The `./agentos` export implements the published sandbox-agent SandboxProvider and agentOS's
AgentOsSandboxProvider. Supply `key` (stable per session), `image` (an explicitly allowed
sandbox-agent server image), and `agentPort`. It uses the real SandboxAgent process client,
including background processes and stdin, rather than pretending buffered exec supports them.
The upstream 0.5.5 Docker provider uses `rivetdev/sandbox-agent:0.5.5-full`; the caller chooses
this image and its host policy. The agent port is private, uses --no-token, and must be reachable
only from trusted actor hosts on the private network. It is never published through the edge.
The native provider starts its server explicitly and uses pause/reconnect for lifecycle. Leave idle unset for background agent work: direct private process traffic is outside the public API activity tracker. Use agentOsProvider as the provider in agentOS's sandbox options; this supplies an external
sandbox/filesystem/process backend, not a replacement for agentOS's JavaScript VM runtime.

[Interface references](https://github.com/cubepals/pryx/blob/main/integrations/interfaces.md) pins the exact source interfaces and their limits. Unit checks
run in the workspace; `bun run integration:e2e` runs the example against Rivet's public self-hosted
engine binary and a real pryx deployment in CI. No Docker/e2e result is claimed locally.

For the actual model/tool loop, run `bun integrations/rivet/example/agent.ts "your coding task"`
while the server is running. Set OPENAI_API_KEY and PRYX_AGENT_MODEL on that client process;
PRYX_AGENT_SESSION reconnects to prior durable memory. PRYX_AGENT_IMAGE defaults to alpine:3.22
and PRYX_AGENT_MAX_STEPS defaults to six. The official OpenAI SDK supplies the model API types.
The loop records the command/key before execution, stores tool results in actor memory, and sleeps
the actor/sandbox afterwards. An unknown exec outcome stops the loop: inspect the recorded key
before requesting another run. Model calls are not part of credential-free CI; CI exercises the
same actor's durable memory, real shell, sleep/wake and destruction against the public engine.
