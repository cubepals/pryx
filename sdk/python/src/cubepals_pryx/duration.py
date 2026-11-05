# SPDX-License-Identifier: FSL-1.1-ALv2
"""Parse positive durations using the generated public contract's units."""
import math, re
from .generated.defaults import SDK_DEFAULTS
def seconds(value):
    match = re.fullmatch(r"(\d+(?:\.\d+)?)([smhd])", value) if isinstance(value, str) else None
    result = float(match[1]) * SDK_DEFAULTS["units"][match[2]] if match else value
    if isinstance(result, bool) or not isinstance(result, (int, float)) or not math.isfinite(result) or result <= 0:
        raise ValueError("Duration is positive seconds or a number followed by s, m, h or d")
    return result
def make_options(options):
    body = dict(options)
    for name in ("ttl", "idle"):
        if name in body: body[name] = seconds(body[name])
    if body.get("image") and "entrypoint" not in body: body["entrypoint"] = list(SDK_DEFAULTS["entrypoint"])
    return body
def run_options(command, options):
    body = dict(options)
    if "timeout" in body:
        body["timeout"] = seconds(body["timeout"])
        if not float(body["timeout"]).is_integer(): raise ValueError("Exec timeout is a whole number of seconds")
        body["timeout"] = int(body["timeout"])
    body["command"] = ["sh", "-c", command] if isinstance(command, str) else command
    return body
