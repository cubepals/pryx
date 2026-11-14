---
title: Connect agents over MCP
---

1. Install the [pryx CLI](cli.md) on the machine running your agent.
2. Launch the agent with `PRYX_API_URL` and `PRYX_API_KEY` in its private environment.
3. Merge this into project `.mcp.json`, restart, and ask for project defaults, a sandbox,
   hello and cleanup. Use an image only when the default tier is container.

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

Choose the file your agent reads: [Claude Code](mcp/claude.md), [Cursor](mcp/cursor.md),
[Codex](mcp/codex.md), or [VS Code and Copilot](mcp/copilot.md). Their variable syntax differs.
A named unavailable isolation or capability is refused.
[MCP tools and transports](../mcp.md), [tier boundaries](../tiers.md).
