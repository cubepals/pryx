# K13: MCP, and Rivet and celld done natively

This is milestone K13 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run.

```
Goal

Make pryx usable by any agent with no SDK, through MCP; make Rivet's agents able to pick pryx
by name; and give celld a backend it can use. Each is a package or a patch at the edge of the
repository that the core never depends on.

Context

Read AGENTS.md, PLAN.md, K9, K11 (the agent inside, `/_pryx/`), K10 (projects and keys),
integrations/ (the K8 packages: the RivetKit actor, the sandbox-agent provider, the agentOS
boundary, the celld proposal and prototype, and integrations/interfaces.md with the commits
read), sdk/typescript/, and the Model Context Protocol specification (modelcontextprotocol.io:
the current revision, stdio and streamable HTTP transports, tools, resources). Then, again at
their current commits, recording each: Rivet's sandbox-agent (its deploy targets, how a target
is implemented and documented, how its server is installed in an image), agentOS's sandbox
boundary, and celld's containers service (its engine, how a node reaches it, its fence image).

What is built:

1. MCP, two ways.
   - `pryx mcp`: an MCP server in the CLI binary, stdio and streamable HTTP, over the public
     API with a key: tools for sandbox create, list, get, delete, run, start, read, write,
     list files, snapshot, fork, sleep, wake, expose, and project usage; resources for a
     sandbox's state and logs. Examples in docs/mcp.md for the common agent clients, by their
     configuration files, without naming vendors beyond the file formats.
   - Per sandbox: pryx-agent serves MCP at `/_pryx/mcp` over streamable HTTP, authenticated by
     the sandbox token, with tools for run, start, files, processes and expose inside that one
     sandbox, so a sandbox can be handed to an agent as one URL. `box.mcpUrl()` and `pryx
     sandbox mcp-url` give it.
   Tests drive both with an MCP client from the specification's reference implementation.

2. Rivet, by name. A template image with sandbox-agent's server installed, built in this
   repository and published with the release, and `pryx template create sandbox-agent` that
   warms it, so a Rivet agent gets a sandbox with its server already up. The provider from K8
   is finished against sandbox-agent's current deploy-target shape and its docs' conventions,
   and the change is prepared as an upstream patch: the files under
   integrations/rivet/upstream/sandbox-agent/, a PR title and body in the project's style,
   and the test they will run; likewise for agentOS's external sandbox boundary. The patch is
   prepared, not sent: sending it is a maintainer's act. The RivetKit actor package moves to
   the K9 handle.

3. celld, two routes, both prepared.
   - The seam upstream: a patch under integrations/celld/upstream/ that adds a container
     engine abstraction to celld with its current local engine as the first implementation and
     a remote one over pryx's API as the second, with celld's own container example passing
     against pryx in CI from a build of the patched celld. Prepared, not sent.
   - The drop-in: pryxd speaks the subset of the Docker Engine API that celld's engine uses, on
     a Unix socket, behind the node feature `docker-api`, off by default: _ping, version,
     images load and inspect, containers create, start, stop, kill, wait, inspect, logs,
     attach, remove, exec create and start, and the events celld watches. A container made
     this way is a sandbox with the host's policy. Proven by celld's unmodified release running
     its container example with DOCKER_HOST pointed at the socket, in CI; measured against
     celld's own engine for start and first request. Shipped behind the flag only if that
     example passes; otherwise kept as a spike with a report in docs/docker-api.md of what
     passed, what didn't, and why.

Constraints

- Nothing in pryxd, control, the edge or the agent depends on any of this; the MCP servers
  are optional surfaces; the Docker API subset is off unless the host turns it on.
- No secret of any project in the repository; every upstream patch builds from the project's
  public release at a recorded commit.
- Nothing is sent anywhere: no pull request, issue or message to another project from inside
  the task. Each patch says, in its body, what it is and how it was tested.
- Title under 50 characters, no motive, no `@`.

Done when

- An MCP client from the reference implementation creates a sandbox, runs a command and reads
  a file through `pryx mcp`, and runs a command through a sandbox's own MCP URL, in CI.
- A Rivet agent runs a command in a pryx sandbox from the warmed template in CI, through the
  finished provider, and the upstream patch directory is complete.
- celld's container example runs on pryx in CI by at least one of the two routes, and
  docs/docker-api.md says which and what it cost.
- `bun run check` passes; CI green.
```
