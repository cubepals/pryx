# pryx documentation site

Astro Starlight reads canonical `docs/` Markdown and both SDK READMEs directly. No second
Markdown tree is generated. Astro, Starlight, Pagefind and parsers are pinned in this
directory's independent lockfile; they stay out of the release image workspace.

Run `bun site/check.ts` from the checkout for dependency setup with scripts disabled, build,
canonical body/coverage verification, local links/anchors and TypeScript checks. `bun run check`
also runs these checks and their negative regressions. Builds and preview disable telemetry.

For local preview use `bun run --cwd site preview --port 4329`; use Astro's printed address.
Stop it with `./node_modules/.bin/astro preview stop` from `site/`.
Open `site/review/index.html` directly for file review. It keeps the generator's rendered
content, styles and relative links; its notice explains that search and interactive navigation
need the full HTTP site. `site/dist/v<version>/` contains that full site and local Pagefind index.
CI's `pryx-site-review` artifact contains both `dist/` and `review/`.

The existing end-to-end job builds this site and runs every connect block against real pryxd
on container and process defaults. Node imports the release SDK build. Python, the real CLI,
the Action's composite shell and the reference MCP client execute the extracted blocks.
Webhook registration saves a private secret file; CI verifies real signed lifecycle deliveries
and removes its registration. Agent account/UI flows and live platform deployments are not claimed.

Publication is off by default. A maintainer may run `bun site/publish.ts` with
`PRYX_SITE_PUBLISH=1`, `PRYX_SITE_URL` and `PRYX_SITE_PUBLISH_DIR` in its environment.
The directory must already exist and be served at that URL's base path by an existing static host.
The command requires a clean checkout at the exact local release tag, builds and checks that
source, verifies its file inventory/checksums and stages an immutable `v<version>/` directory.
It retains previous versions and atomically updates its own root index. It refuses an unowned
index, symlinks, changed versions, untracked public assets and mismatched build addresses.
Nothing in this task enables publication or configures a host, account, DNS, TLS or repository setting.

Set the same optional `PRYX_SITE_URL` on a control service to enable welcome links for its
running version. Image startup forwards it privately. Unset means no invented documentation
hostname. The publisher retains old versions so older running installs keep their guide URLs.
The current release tag predates this source; publish only a future release that includes it.

`bun control/tools/release.ts generate` updates the Action guide from the Cargo-owned version;
the existing release check rejects drift. Generated artifacts retain upstream third-party
licences/notices without copying those packages' implementations into the repository.
