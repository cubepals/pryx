# SPDX-License-Identifier: FSL-1.1-ALv2
"""Run websockify without request logging: WebSocket capability URLs must never enter logs."""
import logging
from websockify.websocketproxy import websockify_init
logging.disable(logging.CRITICAL)
websockify_init()
