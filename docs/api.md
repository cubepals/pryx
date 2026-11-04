# Public API

Authenticate with `Authorization: Bearer <key>`. Create, list and revoke keys with `pryx key` on the private operator listener. The project is chosen by the key. See control/README.md for configuration and sdk/typescript/README.md or sdk/python/README.md for examples.

| Method | Endpoint | Request schema | Success |
| --- | --- | --- | --- |
| POST | /v1/sandboxes | MakeRequest | 201 |
| GET | /v1/sandboxes | — | 200 |
| GET | /v1/sandboxes/{id} | — | 200 |
| DELETE | /v1/sandboxes/{id} | — | 200 |
| POST | /v1/sandboxes/{id}/start | — | 200 |
| POST | /v1/sandboxes/{id}/stop | — | 200 |
| POST | /v1/sandboxes/{id}/sleep | — | 200 |
| POST | /v1/sandboxes/{id}/wake | — | 200 |
| POST | /v1/sandboxes/{id}/rest | — | 200 |
| POST | /v1/sandboxes/{id}/exec | ExecRequest | 200 |
| GET | /v1/sandboxes/{id}/logs | — | 200 |
| PUT | /v1/sandboxes/{id}/files/{path} | — | 200 |
| GET | /v1/sandboxes/{id}/files/{path} | — | 200 |
| POST | /v1/sandboxes/{id}/snapshots | — | 201 |
| GET | /v1/sandboxes/{id}/snapshots | — | 200 |
| DELETE | /v1/sandboxes/{id}/snapshots/{snapshot} | — | 200 |
| POST | /v1/snapshots/{snapshot}/sandboxes | MakeRequest | 201 |
| GET | /v1/sandboxes/{id}/endpoints | — | 200 |
| GET | /v1/sandboxes/{id}/events | — | 200 |
| POST | /v1/templates | TemplateRequest | 201 |
| GET | /v1/templates | — | 200 |

Requests and responses are defined in [the OpenAPI document](../control/openapi/pryx.openapi.json). Exec and logs use application/x-ndjson; exec emits stdout/stderr chunks and a final exit event after the node completes its bounded command. File writes take JSON dataBase64; reads return a base64 tar archive. File paths are relative to persistent storage.

Use a stable Idempotency-Key for retried create or exec requests. Reusing one for different input is refused. An exec with an uncertain outcome is never replayed automatically. Lifecycle operations return the reached sandbox and, for sleep/wake, its tier.

TTL expires access immediately and upkeep deletes data. Idle is evaluated from activity and live sessions. Templates warm each active node once and retain a stopped snapshot. Public endpoints include their edge hostname/port; private endpoints are reachable only on the private network.

Errors have the node envelope `{error:{code,message,details}}`. Invalid fields answer 422, missing keys 401, forbidden operations 403, missing resources 404, state conflicts 409, expired TTL 410 and unavailable placement 503. API v1 evolves additively.
