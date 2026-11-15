---
title: Connect Codex
---

1. Install the [pryx CLI](../cli.md) on the machine running Codex.
2. Start Codex with `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Merge this into `~/.codex/config.toml`, restart, and ask for project defaults, a sandbox,
   hello and cleanup. Supply an image only for a container default.

```toml
[mcp_servers.pryx]
command = "pryx"
args = ["mcp"]
env_vars = ["PRYX_API_URL", "PRYX_API_KEY"]
```

The [Codex reference](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) also documents
project `.codex/config.toml` in trusted projects. `env_vars` passes named values without storing
the key. Host policy and [tier limits](../../tiers.md) remain in force.
