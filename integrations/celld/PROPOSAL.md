<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# A remote Containers backend for celld

Current celld hardcodes ContainerEngine around Docker::discover and a Unix socket. Its V8 ops
call that concrete type. There is no published remote backend trait, HTTP backend API or plugin
registration point. See the pinned [source references](../interfaces.md). This package proposes
an explicit backend boundary; it does not replace ctx.container in an unmodified release.

A future backend should own start, inspect/address, wait, signal/destroy, exec/input/logs and
inactivity operations. Start returns an opaque handle plus a port-to-private-endpoint map; an
address method must support different host ports, rather than assuming a local bridge IP. pryx
would map the cell's ownership generation to a durable create key, set sandbox idle, and sleep on
inactivity. Ownership release must delete or fence the old sandbox, not merely abandon its id.
The backend must refuse unsupported isolation/egress controls instead of weakening celld's
runtime and enableInternet policy. Image delivery needs an explicit OCI registry workflow:
celld's saved image tar is not a registry reference. Do not interpret the prototype as support for
its image-import, arbitrary TCP socket, monitor/streaming-process or runtime-selection surfaces.

The prototype runs an ordinary celld Worker/Durable Object from the current public release.
Its DO persists the sandbox id/generation and calls an authenticated private bridge. The bridge
makes a sandbox on first use through the public SDK, sets idle, wakes before forwarding HTTP,
and explicitly sleeps/deletes it when asked. Deletion advances the generation so a new use cannot
receive a cached deleted identity. It strips its own credential and does not follow redirects.
The private destination comes only from the SDK's endpoint response, never a client-supplied URL.

CI runs this actual Worker on celld dev, forwards a request to a real sandbox, sleeps it, confirms
the pryx state and wakes it on another request. That demonstrates the transport/lifecycle half
that needs no upstream changes. Native @cloudflare/containers support requires an upstream backend
boundary with the operations above; no source patch or unsupported backend claim is hidden here.
The bridge is a prototype, binds loopback in the test, and requires separate worker/operator
authentication before deployment. Nothing in pryx's core imports this package.
