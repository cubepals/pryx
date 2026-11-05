<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

Add a remote container engine boundary

Separate container effects from the current ownership and inactivity coordinator. Keep the
existing local Docker implementation and add a prepared pryx public-API implementation with
private TCP endpoints, host-enforced isolation and a durable creation journal.

The prepared remote backend supports the single-node development store and refuses distributed
ownership, local OCI runtime or DNS overrides, remote exec and root-process signaling. An uncertain
creation blocks replacement until reconciled. Distributed deployment needs a remote fencing
handoff before this backend can be enabled there.

Prepared against public release 0.6.3 at commit 8702f6d186c77698dbfb30cf29e3b1922a88f008.
Locked cargo check and the patched executable build pass locally; the binary reports release
0.6.3. The unchanged upstream container example remains assigned to CI, with no local success claim. CI collects 30 start and
HTTP samples for both engines on the same host. No upstream pull request, issue or message was sent.
