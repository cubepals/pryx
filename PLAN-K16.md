# K16: the surface the field has

This is milestone K16 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run. It is built
after K15, on K15's one surface, and every addition below is sugar a user of E2B, Daytona or
Cloudflare's sandbox SDK already expects by name.

```
Goal

Give both SDKs, the CLI and the public API the conveniences agent builders reach for first,
so that nobody who has used another sandbox SDK finds a method missing: waiting on a process,
a stateful shell, git, file search and replace, upload and download to disk, labels and
lookup, lifetime control, metrics, templates built from code, an egress allowlist, and
lifecycle webhooks.

Context

Read AGENTS.md, PLAN.md, K11 (the agent's API), K15 (the one surface), the SDKs, cli/src/,
control/src/ (the public API, templates, upkeep, events), agent/src/ (processes, files,
ports), docs/api.md and docs/agent.md. Each item names where it lives: agent (inside the
sandbox, no control involved), control (the public API), or SDK (sugar over either).

What is added:

1. Processes (agent, SDK). On the Process handle: `waitForPort(port, {mode: 'http'|'tcp',
   path, status, timeout})`, `waitForLog(pattern, timeout)`, `waitForExit(timeout)`, `logs()`;
   on the sandbox: `processes()`, `killAll()`; `run` and `spawn` take `stdin: string | bytes`
   for one-shot input. A process exited before ready and a timeout are distinct errors.
2. Shell sessions (agent, SDK). `const sh = await box.shell({cwd, env})`: a long-lived shell
   where `sh.run(cmd)` keeps cwd, env and shell state between commands and returns the same
   result shape as `box.run`; `sh.close()`. Implemented over the agent's PTY or a pipe-driven
   shell with a sentinel per command, whichever gives exact exit codes and stderr; measure and
   say which.
3. Files (agent, SDK). `files.exists`, `files.find(glob)`, `files.grep(pattern, {paths})`,
   `files.replace(pattern, text, {paths})`, `files.chmod`, `files.upload(localPath, remote,
   {onProgress})`, `files.download(remote, localPath, {onProgress})`, `files.uploadDir` and
   `downloadDir` as tars, `box.sync(localDir, remoteDir)` by content hash. The CLI's `cp` uses
   the same.
4. Git (SDK over the agent's processes). `box.git.clone(url, {path, branch, depth, token})`,
   `status`, `diff({staged})`, `branch`, `checkout(ref, {create})`, `add`, `commit(message,
   {author})`, `push({token})`, `pull`, `log({limit})`, each returning parsed values, with the
   raw output kept on the result; the token is passed through a credential helper for one
   call, never written to disk in the sandbox. `pryx.sandbox({repo: url, branch, token})`
   clones before the sandbox answers.
5. Create options (control, SDK). `env`, `labels`, `name`, `workdir`, `user`, `autoDelete`
   (delete after idle), `egress: {allow: [hosts], deny: [hosts]}` (a node feature on the
   container tier, enforced on the sandbox's bridge by hostname resolved at connect time,
   announced before use; refused on tiers that cannot enforce it, with the tier named), and
   `metadata` as a free object. `pryx.sandboxes({labels, state, name})` filters;
   `pryx.sandbox({name})` finds by name.
6. Lifecycle (control, SDK). `pryx.sandbox(id)` on a sleeping or resting sandbox wakes it
   before returning (option `wake: false` to inspect without waking); `box.keepAlive(duration)`
   and `box.setTtl`, `box.setIdle`; `box.metrics()` (cpu, memory, disk, network as the node
   last reported, with `since` for a series); `box.waitUntil(state, timeout)`; `box.events()`
   as an async iterator with `follow`.
7. Templates from code (control, SDK, CLI). `pryx.template({name, from: image, copy: [[local,
   remote]], run: [commands], env, workdir, ready: command, start: command})` builds on a node,
   snapshots after `ready` succeeds, and names the result; `pryx templates`, `pryx template
   build -f pryx.template.ts`; a sandbox from a template starts with `start` running.
   Versions are kept; `template.rebuild()`.
8. Webhooks (control). `POST /v1/webhooks` with a URL and the event kinds; control delivers
   sandbox lifecycle events (created, running, sleeping, woken, resting, deleted, failed,
   ttl expired, idle slept) signed with a secret shown once, with retries and a dead-letter
   list; `pryx webhook create|ls|rm`; documented in docs/api.md with the signature scheme.
9. The CLI gets a command for each of the above where one makes sense, and `pryx sandbox
   shell` as the interactive form of 2.

Constraints

- Public API additive within v1; OpenAPI committed; every new agent route behind the agent's
  version in its health answer; every node-side change behind a node feature.
- Nothing above weakens isolation: git tokens never persist in the sandbox; egress rules are
  enforced on the host, never trusted from inside; webhooks carry no secrets.
- Unit tests per item against a fake transport in both SDKs; the end-to-end suite in CI takes
  one sandbox through every item in order, both SDKs, and the CLI through the commands added.
- Each SDK README's surface table grows by exactly the rows added here and stays under 200
  lines.
- Title under 50 characters, no motive, no `@`.

Done when

- A test in CI starts a web server with `spawn`, `waitForPort`s it, `expose`s it and fetches
  a 200 through the hostname; a `shell` keeps `cd` across two commands; `git.clone` of a public
  repository then `git.status` reads clean; `files.grep` finds a known line; a template built
  from code starts a sandbox with its `start` command running; a webhook receives `created`
  and `deleted` with a valid signature; an `egress.allow` of one host lets that host through
  and refuses another.
- docs/api.md and both SDK READMEs document every addition in the surface table.
- `bun run check` passes; CI green.
```
