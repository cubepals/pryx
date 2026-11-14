# Prepared OpenOrange provider registration

Prepared only: no upstream pull request, issue or comment was opened. OpenOrange 1.0.2 does
not yet offer `pryx`; an unmodified release cannot select this provider. This patch targets
Apache-2.0 OpenOrange commit `1cabf2e9db300361c9f0b751c7723da7eb706f5e`, read on 2026-11-14.
`source.json` pins the exact original files. No OpenOrange implementation is shipped in pryx.

The proposed change adds `ORANGE_SANDBOX_PROVIDER=pryx`, lazy SDK imports and validation of
`PRYX_API_URL`/`PRYX_API_KEY`. Its Actor continues to store provider name and sandbox id. The
public SDK owns lifecycle, execution and file access. The binding uses the connected sandbox's persistent directory and
native binary files and directory metadata, instead of requiring GNU file tools. Recursive
list never descends into symlinks. Preview requests call `pryxPortAddress` and use pryx's edge,
retaining request bodies, paths, query parameters and the edge URL's prefix. Git defaults to
the backend's working directory. Other providers keep their existing working directories.

To offer this upstream, apply `change.patch` to that commit and install the SDK release
containing `@cubepals/pryx/provider` next to Orange. Add that same released SDK as an optional
peer and a pinned development dependency for Orange's typecheck. Extend Orange's provider
settings documentation and tests with the name, URL/key requirements, missing-sandbox
reconnection, native byte writes and preview forwarding. Then run its full typecheck and test
suite before publishing. That upstream suite has not run here. The existing OpenOrange
adapter bridge maps combined output to stdout; this patch retains that behavior. Complete
`Files.stat/lstat` metadata stays partial. Session/code-context and container-only methods
keep their existing refusals.

pryx's existing CI e2e job applies this exact patch to digest-verified upstream files, typechecks and builds
the registration and exercises its create/connect, binary files, directory listing, execution,
edge preview and destroy against the same real pryx deployment as the SDK provider. It does
not bundle or run OpenOrange or Rivet's control plane in a pryx installation. The local command
`bun control/tools/open-orange-patch.ts` only prepares ignored validation output.

The browser binding is deliberately unchanged: pryx's short-lived, rotating browser
capability cannot be a fixed `ORANGE_BROWSER_CDP_URL`. A future refresh contract would be
needed; the current bridge makes no Browser Rendering claim.
