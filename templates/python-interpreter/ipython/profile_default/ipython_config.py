# SPDX-License-Identifier: FSL-1.1-ALv2
"""Inline plotting and structured values are kernel configuration, not a second interpreter."""
c = get_config()  # noqa: F821: the owner, IPython, supplies this config factory
c.HistoryManager.hist_file = ':memory:'
c.InlineBackend.figure_formats = {'png', 'svg'}
c.InteractiveShellApp.matplotlib = 'inline'
c.InteractiveShellApp.exec_lines = [
    'import numpy as np; import pandas as pd; import matplotlib.pyplot as plt',
    'from IPython import get_ipython',
    'from IPython.core.formatters import JSONFormatter',
    "get_ipython().display_formatter.formatters['application/json'] = JSONFormatter(parent=get_ipython().display_formatter)",
    "get_ipython().display_formatter.formatters['application/json']._return_type = (dict, list, int, float, bool)",
    "[(get_ipython().display_formatter.formatters['application/json'].for_type(t, lambda value: value)) for t in (dict, list, int, float, bool)]",
    "get_ipython().display_formatter.formatters['application/json'].for_type(pd.DataFrame, lambda frame: __import__('json').loads(frame.to_json(orient='records')))",
]
