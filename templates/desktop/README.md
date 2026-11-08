# Desktop image

[Dockerfile](Dockerfile) inherits the versioned native-tools image; its layer and
package licences are listed in [the native-tools README](../native-tools/README.md).
Its only added configuration selects the pryx desktop launcher (FSL-1.1-ALv2).
Xvfb, JWM, x11vnc and noVNC remain external programs with their installed notices.

The native layer also installs [requirements.txt](requirements.txt) with hashes:
websockify 0.13.0 and jwcrypto are external LGPL-3.0 packages, while the resolved
NumPy, cryptography and other wheels retain their respective BSD, Apache-2.0,
MIT and Python notices. Every wheel version, hash and full-notice location is in
[the Python inventory](../../docs/checks/measurements/k18-desktop-python-licences.json).
Legal text remains in the installed wheels under `/usr/local/lib/python3.12/site-packages`.
Corresponding upstream sources are linked from that inventory and the wheels'
metadata; no LGPL implementation is copied here.

The launcher runs as the sandbox user, binds VNC and websockify to loopback, and
uses expiring per-session preview tokens. See [native-tools.md](../../docs/native-tools.md)
for the SDK and deployment configuration.
