# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bind websockify's maintained Unix target plugin to the current expiring capability."""
import hmac
import json
from pathlib import Path
import time
from websockify.token_plugins import TokenFile


class ExpiringTokenFile(TokenFile):
    def lookup(self, token):
        try:
            access = json.loads(Path(self.source).with_name('desktop-access.json').read_text())
            if access['expiresAt'] <= time.time() or not hmac.compare_digest(access['token'], token):
                return None
            return super().lookup(token)
        except (OSError, ValueError, KeyError, TypeError):
            return None
