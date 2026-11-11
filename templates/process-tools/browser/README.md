# Process browser adapter

Puppeteer Core owns Chromium's debugging-pipe framing, CDP IDs and sessions; ws owns WebSocket
framing. The adapter maps each authorized client to its own browser-target session, preserves
upstream errors and serves over an abstract Unix listener. Chromium's sandbox is mandatory.
The host installs the browser-only policy and private proc namespace before this program starts.

Parts (`browser/`):

- `package.json`, `package-lock.json`: reviewed integrity-pinned external programs.
- `bridge.mjs`: browser-target sessions over the maintained CDP connection.
- `start.mjs`: pipe launch, expiring discovery/proxy and private screenshots.
- `smoke.mjs`: real screenshots, CDP expiry and observed renderer confinement.
