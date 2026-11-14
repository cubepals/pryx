# Network comparison programs

These programs wrap gvisor-tap-vsock for the K25 comparison and closed runtime profile. They
stay separate from the kernel fixture's Go module so dependency selection cannot change its
closure. The runtime profile remains unwired and enables no VM tier or guarantee by itself.

Parts (`network/`):

- `go.mod` and `go.sum`: pinned network library and unchanged upstream guest-forwarder tool.
- `go.work` and `go.work.sum`: make both local fixture modules explicit for cache verification within this directory only.
- `cmd/proxy/main.go`: separate guest packet transport and private host management listeners.
- `cmd/proxy/production.go`: bounded host-only profile through one inherited packet listener, without management services.
- `cmd/guest/main.go`: trusted loopback and maintained DHCP setup without image-owned programs.
- `cmd/probe/main.go` and `server.go`: guest network measurements, management denials and an unprivileged opaque TCP fixture.
- `cmd/probe/policy.go`: unprivileged workload proof that new vsock sockets and io_uring setup are denied.
- `network.go`: the upstream API's unexported route names, with their source recorded.
- `inputs.json`: immutable Alpine and pasta source inputs, plus Alpine's actual package licences/source commits.
- `module-notices.json`: the exact dependency closure's complete primary-notice identities.

The gateway management token stays in host memory. The guest-facing Unix listener accepts
only the library's packet-connect route; mapping its unrestricted host Mux into vsock would
also expose management. The library owns every frame and TCP/IP operation.
