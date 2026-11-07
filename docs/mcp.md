<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# MCP

`pryx mcp` exposes this project's sandboxes through stdio. Set PRYX_API_URL and PRYX_API_KEY
in the process environment. Standard output contains only MCP messages. The optional command
`pryx mcp --http 8090` instead serves stateless Streamable HTTP on
`http://127.0.0.1:8090/mcp`; every request needs `Authorization: Bearer <project-key>`.
The HTTP listener checks Host and Origin and binds loopback.

For a `.mcp.json` configuration:

```json
{
  "mcpServers": {
    "pryx": {
      "command": "pryx",
      "args": ["mcp"],
      "env": {
        "PRYX_API_URL": "https://pryx.example.com",
        "PRYX_API_KEY": "replace-with-project-key"
      }
    }
  }
}
```

For a `config.toml` configuration:

```toml
[mcp_servers.pryx]
command = "pryx"
args = ["mcp"]
[mcp_servers.pryx.env]
PRYX_API_URL = "https://pryx.example.com"
PRYX_API_KEY = "replace-with-project-key"
```

An HTTP configuration uses the same project's key:

```json
{
  "mcpServers": {
    "pryx": {
      "type": "http",
      "url": "http://127.0.0.1:8090/mcp",
      "headers": { "Authorization": "Bearer replace-with-project-key" }
    }
  }
}
```

Tools cover create, list, get, delete, run, background start, read, write, list files,
snapshot, fork, sleep, wake, expose and project usage. State and logs are resources under
`pryx://sandboxes/<id>/state` and `pryx://sandboxes/<id>/logs`. All calls use the public SDK
and retain project admission and isolation. Tool errors are explicit; uncertain commands are
never resubmitted. File reads are limited to 1 MiB; use SDK streaming for larger files.

A single sandbox also has its own MCP endpoint. Obtain its expiring URL with
`pryx sandbox mcp-url <id>`, `await box.mcpUrl()` in TypeScript, or `box.mcp_url()` in Python
(`await box.mcp_url()` for the async handle). The URL contains a scoped sandbox capability;
put it in an HTTP client's URL field. It permits only that sandbox, expires on the existing
agent token schedule, and can be regenerated when needed. It contains no project key.
Its tools cover run, background processes, stdin, signals, files and exposed ports;
resources are `pryx://sandbox/state` and `pryx://sandbox/logs`. Browser Origin requests are
refused. The agent can be built without this optional surface using
`cargo build -p pryx-agent --no-default-features --features server`.

Both surfaces negotiate MCP 2025-11-25 with the released TypeScript reference SDK 1.32.2.
Current 2026-07-28 clients use the specification's legacy-server fallback. No modern protocol
revision or unsupported capability is advertised. Tests use the actual reference client over
both CLI transports; CI additionally runs real commands and reads files through the CLI and
sandbox endpoints. The real-container acceptance has not run locally.
