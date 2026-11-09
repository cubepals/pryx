# K23: browsers and kernels without Docker

This is milestone K23 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run. Licence first, as in K17 and
K18: every program in the bundle is checked and recorded before use, and nothing copyleft is
linked or copied.

```
Goal

A browser an agent can drive and a live language kernel work on the process tier, so a person
who installed with one command and no Docker has both. The same SDK calls as today:
`box.browser()`, `box.browser.screenshot(url)`, kernels and `run_python`. Where the browser's
own sandbox can't be had, pryx says so and refuses where nothing else holds the browser.

Context

Read docs/any-host.md sections 3.3 and 4.5, K19's guarantees, docs/native-tools.md,
docs/interpreter.md, agent/src/tools/ (state, browser), agent/src/kernels/ (registry and its
refusal on shared-host tiers), agent/src/process_policy.rs and process_policy/filter.rs,
templates/browser/, templates/python-interpreter/, templates/node-interpreter/,
templates/native-tools/debian.lock.json and its licence inventories, deploy/
native-tools-seccomp.json. Then, at recorded versions and licences: Chrome for Testing's
headless shell and Playwright's use of `--remote-debugging-pipe`; the Jupyter protocol's `ipc`
transport in jupyter_client and ipykernel; conda-forge's packages for the browser's libraries
and fonts, and micromamba as a build-time tool.

What is built

1. The tool bundle: a relocatable directory with the headless shell, the libraries it needs,
   fonts with CJK and emoji coverage, CPython with ipykernel and the interpreter template's
   libraries, Node with its kernel. Built in CI from pinned packages against an old glibc,
   with the same licence inventory the templates have, published with the release, verified
   by digest, and fetched by pryxd the first time a sandbox on a tier without images asks for
   a tool. The container templates are unchanged.
2. Private transports on the process tier. The agent drives the browser over the debugging
   pipe and serves the same authenticated, expiring proxy as today. Kernels use `ipc`
   connection files and sockets under the sandbox's own directory. Nothing listens on TCP.
   Landlock already keeps other sandboxes out of that directory; a test proves a neighbour
   can't open the sockets, in shared-uid mode too.
3. The process tier's policy for tools: its seccomp filter gains a variant, used only for the
   browser's process tree, that lets Chromium create its own namespaces where the host allows
   them; Landlock stays as it is and is inherited.
4. The browser's sandbox. On wherever the probe proved user namespaces. Where it can't be: on
   the container tier the browser runs without it and the sandbox's `guarantees` report
   `browser-sandbox` off (today's refusal becomes a report, since the container holds it); on
   the process tier it is refused with the reason and what would turn it on. A create with
   `require: ['browser-sandbox']` is placed only where it is on.
5. The refusals in agent/src/tools/state.rs and agent/src/kernels/registry.rs become per
   capability, from what the tier can give, not per tier by name. Wasm keeps refusing both.
6. docs/native-tools.md and docs/interpreter.md say where each tool runs and what it needs;
   docs/compare.md's cells follow.

Constraints

- No listener a neighbouring sandbox can reach, in any uid mode; no abstract sockets unless
  the probe proved Landlock scopes them (ABI 6).
- The bundle is never fetched unasked, never from an unpinned address, and never executed
  before its digest is checked.
- No `--no-sandbox` on the process tier under any flag or setting.
- The container tier's hardening and its measured budgets stay.
- Title under 50 characters, no motive, no `@`.

Done when

- On the `user` preset in CI: a browser screenshot of a page and a CDP session through the
  edge; a Python and a JavaScript kernel with state kept between runs and a chart returned;
  each with 30 timed samples in docs/speed.md beside the container tier's.
- In a default `docker run` of the image (no user namespaces): kernels work; the browser is
  refused on the process tier with the message asserted.
- The neighbour test and the guarantee tests pass; both SDKs' existing tool tests run on the
  process tier too.
- `bun run check` and `cargo test --locked` pass; CI green on the pull request and on main.
```
