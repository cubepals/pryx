---
title: Connect Cursor
---

1. Install the [pryx CLI](../cli.md) on the machine running Cursor's MCP process.
2. Start Cursor with `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Merge this into `.cursor/mcp.json`, restart, and ask for project defaults, a sandbox,
   hello and cleanup. Supply an image only for a container default.

```json
{
  "mcpServers": {
    "pryx": {
      "type": "stdio", "command": "pryx", "args": ["mcp"],
      "env": { "PRYX_API_URL": "${env:PRYX_API_URL}", "PRYX_API_KEY": "${env:PRYX_API_KEY}" }
    }
  }
}
```

[Cursor's reference](https://cursor.com/docs/mcp) owns this file and variable syntax. Keep
literal credentials out of it. Requests retain the host's [tier refusals](../../tiers.md).
