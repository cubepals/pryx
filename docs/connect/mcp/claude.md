---
title: Connect Claude Code
---

1. Install the [pryx CLI](../cli.md) where Claude Code runs.
2. Start Claude Code with `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Merge this into project `.mcp.json`, restart, and ask for project defaults, a sandbox,
   hello and cleanup. Container defaults need an image; process and Wasm must omit it.

```json
{
  "mcpServers": {
    "pryx": {
      "type": "stdio", "command": "pryx", "args": ["mcp"],
      "env": { "PRYX_API_URL": "${PRYX_API_URL}", "PRYX_API_KEY": "${PRYX_API_KEY}" }
    }
  }
}
```

[Claude Code's reference](https://code.claude.com/docs/en/mcp) defines this file and variable
expansion. Host admission enforces [tier limits](../../tiers.md); an agent cannot grant missing capabilities.
