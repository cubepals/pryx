# Native desktop worker

This closed worker starts unchanged Xvfb, Chromium, xdotool, scrot, x11vnc,
noVNC and websockify. Hosted worker and installed SDK/edge acceptance track integration
in the progress record; the bundle and corresponding sources have no live release yet.

Parts (`desktop/`):

- `start.mjs`: program supervision, private command access and scoped HTTP forwarding.
- `vnc.py`: pass private Unix connections to x11vnc's maintained inetd interface.
- `web.py`: run maintained websockify with capability URL logging disabled.
- `policy.py`: bind the existing token-file resolver to wall-clock expiry.
- `protocol.json`: private adapter paths and websockify's documented Unix target marker.
- `ssh-shell`: enter the existing embedded shell with inherited guest restrictions.
