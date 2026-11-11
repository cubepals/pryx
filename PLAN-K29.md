# K29: the public site and its connect pages

This is milestone K29 of PLAN.md. It follows K28. Everything under "How to run this plan" in
PLAN.md applies to it: read AGENTS.md first, above all "How pryx decides"; work autonomously,
never ask, record choices in `docs/decisions.md`, commit with short subjects and no `@`, never
claim a check passed that did not run. Licence first, as before.

```
Goal

One public site where a person learns what pryx is, hosts it with one click, and connects
whatever they have to it. Each connect page is short: three steps and one block to copy.

Context

Read README.md, docs/README.md and every page under docs/, K28 (the welcome page, the
platform files, docs/hosting.md), both SDK READMEs, docs/mcp.md, docs/api.md, action/action.yml,
docs/templates.md. Then, at recorded versions, the documentation and licence of the site
generator chosen: Astro Starlight first; VitePress if its licence or upkeep rules it out.
The reason for the choice is recorded.

What is built

1. A static site under site/, built from the repository's own docs so there is one copy of
   every page. Light and dark, readable on a phone, searchable without a service.
2. A front page: what pryx is in one screen, the one command, and the hosting buttons from
   K28.
3. Connect pages, one per caller, each three steps and one block: GitHub Actions; agents
   over MCP, with the configuration file each common agent reads; a TypeScript backend, with
   a note for serverless platforms; Python; the CLI; webhooks; plain HTTP.
4. Host pages: one per platform from docs/hosting.md, and one for a machine of your own,
   each ending at the welcome page.
5. The welcome page from K28 links to the connect pages of the version that is running.
6. The site builds in CI on every pull request. Publishing it is a switch a maintainer
   turns on; the address is a variable, so a new name or domain is one change.

Constraints

- Wrap a maintained generator; no hand-rolled site framework. Licence recorded first.
- No page says what the code doesn't do. A capability a tier or host lacks is said on the
  page that mentions it.
- No analytics, tracker, or script or font fetched from another origin.
- No price, competitor comparison beyond docs/compare.md, or name of a person.
- The repository's settings, its Pages configuration and any domain are not changed.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI: the site builds with no broken link; every block on every connect page is
  extracted and run against a real pryx started in the same job; a block that stops working
  fails the build.
- A check fails when a page under docs/ is missing from the site or differs from it.
- The built site is uploaded as a CI artifact that opens from a file, for review before
  anything is published.
- `bun run check` passes; CI green on the pull request and on main.
```
