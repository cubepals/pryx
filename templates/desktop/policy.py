# SPDX-License-Identifier: FSL-1.1-ALv2
"""Add wall-clock expiry to websockify's unmodified TokenFile resolver."""
import hmac
import json
import pathlib
import time
from websockify.token_plugins import TokenFile

class ExpiringTokenFile(TokenFile):
    def lookup(self, token):
        try:
            access = json.loads(pathlib.Path(self.source).with_name("desktop-access.json").read_text())
            if access["expiresAt"] <= time.time() or not hmac.compare_digest(access["token"], token):
                return None
            return super().lookup(token)
        except (OSError, ValueError, KeyError, TypeError):
            return None
