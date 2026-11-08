# Browser image

The donor is the official Playwright 1.65.0 Ubuntu noble image, pinned to its
amd64 digest in [Dockerfile](Dockerfile). Its seven audited layers and 515
package notices are recorded in [the image inventory](../../docs/checks/measurements/k18-playwright-licences.json);
Ubuntu packages retain their licences under `/usr/share/doc`.

The final image contains Chromium headless shell revision 1254, version
158.0.8127.18 (BSD and retained third-party notices), and FFmpeg revision 1013
(LGPL-2.1 and retained notices). The donor's full Chromium includes proprietary
Widevine: that tree, Firefox and WebKit are removed before the sanitized filesystem
is copied into a scratch final image. Thus no published layer contains Widevine.
The headless shell keeps Chromium's sandbox enabled.

[package-lock.json](package-lock.json) pins playwright-core 1.65.0 (Apache-2.0),
http-proxy 1.18.1 and its MIT dependencies. All resolved package licences and
notices are retained in [the npm inventory](../../docs/checks/measurements/k18-browser-licences.json)
and `/opt/pryx/browser/node_modules`. The launch and proxy wrappers are
FSL-1.1-ALv2; browser and proxy listeners bind to loopback and preview access
requires an expiring token. See [native-tools.md](../../docs/native-tools.md).
