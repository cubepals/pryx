# Edge

Run `bun edge/src/agent.ts` with the edge token shared with control. TLS SNI, HTTP/1.1 Host and
raw TCP port fronts forward original bytes without TLS termination or application messages.
The sandbox owns its TLS certificate. HTTP and TLS fronts bind only after the first route list
is applied. Raw ports and extra TLS listeners are added individually; updates retain other routes.

The private network must allow the edge to reach control's API listener and the node's allocated
ports, and allow only edge addresses in pryxd's `network.edge_ips`. Control alone reaches its
management API and control-audience ports. Keep ops/metrics private. A node needs no public
address. Do not expose Docker, Postgres or S3 administration to the internet.

| Environment | Default |
| --- | --- |
| PRYX_CONTROL_URL | http://127.0.0.1:8082 |
| PRYX_EDGE_TOKEN | required; same token in control, at least 32 characters |
| PRYX_EDGE_ID | OS hostname; unique per edge |
| PRYX_EDGE_HOST | 0.0.0.0 (public fronts) |
| PRYX_EDGE_HTTP_PORT | 80 |
| PRYX_EDGE_TLS_PORT | 443 |
| PRYX_EDGE_METRICS_HOST | 127.0.0.1 |
| PRYX_EDGE_METRICS_PORT | 9090 |
| PRYX_EDGE_HOLD_SECONDS | 5, from the measurements in docs/edge.md |
| PRYX_EDGE_GREETING_SECONDS | 5 |
| PRYX_EDGE_BUFFER_BYTES | 65536, across greeting and hold |
| PRYX_EDGE_REQUEST_SECONDS | 3, each control request |
| PRYX_EDGE_ADDRESS_CONNECTIONS_PER_SECOND | 5 |
| PRYX_EDGE_ADDRESS_CONNECTIONS_BURST | 30 |
| PRYX_EDGE_ADDRESS_GREETING | 10 |
| PRYX_EDGE_ADDRESS_OPEN_CONNECTIONS | 64 |
| PRYX_EDGE_SERVER_CONNECTIONS_PER_SECOND | 20 |
| PRYX_EDGE_SERVER_CONNECTIONS_BURST | 100 |
| PRYX_EDGE_SERVER_OPEN_CONNECTIONS | 300 |

Control's `PRYX_EDGE_PORT_RANGE` defaults to `20000-20100`. A public port's `front` is http, tls or
port; default http (tls when `tls: true`). `tls: true` uses the spec's port as the extra public
TLS port; otherwise TLS uses 443. Use aliases to give multiple HTTP/TLS ports distinct hostnames.
Raw port allocations survive moves and wait ten minutes after deletion before reuse. Public UDP
is refused; private UDP remains supported by the node. No PROXY header is added.

Connections waiting for one sandbox share a wake request. Leaving one connection does not cancel
another's wake. No bytes are sent to a waiting client; ingress buffering and the monotonic hold
are bounded. After splice, the hold timer no longer governs the connection. A higher epoch
closes older connections, even if a port/address is reused, and delayed older routes are ignored.
Route deletion closes its connections. Session refreshes every 30 seconds keep control's idle
policy aware of long-lived connections; stale sessions expire after two minutes.

The private /metrics listener reports routes/version held, a route-freshness lag indicator,
current holds, wake answers/outcomes, refusals by limit, report failures and stale closes. Labels
never contain an address or sandbox id. Failed polling keeps held routes; failed wakes close the
waiting connection within its hold. For a one-host deployment leave control on loopback and
point PRYX_CONTROL_URL at it. Run `bun run check` for unit tests; real daemon e2e belongs to CI.
