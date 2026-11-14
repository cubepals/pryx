---
title: Connect to your install
---

Open your install's welcome page to find its key location and reported tier. Keep the key in
your caller's private environment. Each guide has three steps and one block to copy.

- [TypeScript backend](connect/typescript.md), including serverless callers
- [Python](connect/python.md)
- [CLI](connect/cli.md)
- [GitHub Actions](connect/action.md)
- [Agents over MCP](connect/mcp.md): [Claude Code](connect/mcp/claude.md),
  [Cursor](connect/mcp/cursor.md), [Codex](connect/mcp/codex.md),
  [VS Code and Copilot](connect/mcp/copilot.md)
- [Webhooks](connect/webhooks.md)
- [Plain HTTP](connect/http.md)
- [Rivet harness](connect/rivet.md)

A container request needs an image and an engine. Process and Wasm refuse container images.
Process requires Landlock/seccomp; a shared UID requires mutually trusted workloads, and
internet-enabled processes share host networking. Wasm uses a virtual kernel and shared sidecar,
with HTTP-only guest ingress. See [tiers](tiers.md) and [hosting boundaries](hosting.md#tiers-and-guarantees).
