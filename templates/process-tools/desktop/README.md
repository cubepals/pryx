# Native desktop worker

This closed worker starts unchanged Xvfb, Chromium, xdotool, scrot, x11vnc,
noVNC and websockify. It is staged for explicit hosted acceptance; installed
desktop capability announcement remains off until integration passes.

Parts (`desktop/`):

- `start.mjs`: program supervision, private command access and scoped HTTP forwarding.
- `vnc.py`: pass private Unix connections to x11vnc's maintained inetd interface.
- `web.py`: run maintained websockify with capability URL logging disabled.
- `policy.py`: bind the existing token-file resolver to wall-clock expiry.
