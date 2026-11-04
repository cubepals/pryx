<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# celld container prototype

Read [PROPOSAL.md](PROPOSAL.md) for the missing upstream interface and supported surface.
`containerBridge(client, {token,image,port,idle,spec})` returns a Hono app. Serve it on a private
loopback/VPN listener. Its bearer token belongs only in the celld server environment. The Worker
in prototype/ persists one sandbox id per DO, creates on first request, proxies HTTP, and accepts
/sleep and /delete for the example's lifecycle. Set PRYX_BRIDGE_URL and PRYX_BRIDGE_TOKEN as celld
Worker vars/secrets; do not commit values. The image must already be allowed by the node's policy.

`bun run integration:e2e` runs the Worker with a pinned public celld release in CI, together with
real pryx/Docker/Postgres/S3. Local unit tests verify the SDK transport, identity and credential
stripping. No native ctx.container backend or local full end-to-end success is claimed.
