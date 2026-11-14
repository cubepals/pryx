# SPDX-License-Identifier: FSL-1.1-ALv2
"""Run maintained websockify without logging capability URLs."""
import logging
from websockify.websocketproxy import websockify_init

logging.disable(logging.CRITICAL)
websockify_init()
