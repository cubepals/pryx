# Network comparison programs

These programs wrap gvisor-tap-vsock for the K25 network spike. They are separate from the
kernel fixture's Go module so their dependency selection cannot change its dependency closure.
No production backend or VM guarantee is enabled here.

Parts (`network/`):

- `go.mod` and `go.sum`: pinned network library and unchanged upstream guest-forwarder tool.
- `go.work`: make both local fixture modules explicit for cache verification within this directory only.
- `cmd/proxy/main.go`: separate guest packet transport and private host management listeners.
- `cmd/probe/main.go`: guest DHCP/HTTP/throughput/DNS measurements and two management denials.
- `network.go`: the upstream API's unexported route names, with their source recorded.
- `inputs.json`: immutable Alpine and pasta source inputs, plus Alpine's actual package licences/source commits.
- `module-notices.json`: the exact dependency closure's complete primary-notice identities.

The gateway management token stays in host memory. The guest-facing Unix listener accepts
only the library's packet-connect route; mapping its unrestricted host Mux into vsock would
also expose management. The library owns every frame and TCP/IP operation.
