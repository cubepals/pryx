---
title: Connect VS Code and Copilot
---

1. Install the [pryx CLI](../cli.md) where the Copilot MCP process runs.
2. Start VS Code with `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Merge this into `~/.copilot/mcp-config.json`, restart the native agent harness, and ask for
   project defaults, a sandbox, hello and cleanup. Use an image only for containers.

```json
{
  "mcpServers": {
    "pryx": {
      "type": "stdio", "command": "pryx", "args": ["mcp"], "tools": ["*"],
      "env": { "PRYX_API_URL": "${PRYX_API_URL}", "PRYX_API_KEY": "${PRYX_API_KEY}" }
    }
  }
}
```

The current [VS Code reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)
defines this native-harness file; `COPILOT_HOME` can relocate its directory. It also describes
shared project configurations. Host [tier boundaries](../../tiers.md) still apply.
