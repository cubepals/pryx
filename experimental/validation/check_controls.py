# SPDX-License-Identifier: FSL-1.1-ALv2
"""Require distinct diagnostics for an incorrect predicate and a correct predicate with an extra read."""
import hashlib
import json
from pathlib import Path
import sys

import angr

from prove import check


def verify(elf):
    project = angr.Project(str(elf), auto_load_libs=False)
    result = {}
    controls = {
        "control_wrong_single_byte": "wrong predicate at length 1",
        "control_extra_load": "memory bounds at pc ",
        "control_discarded_load": ("unmodeled Arm memory read at pc " if project.arch.name == "AARCH64"
                                   else "memory bounds at pc "),
    }
    for name, diagnostic in controls.items():
        symbol = project.loader.main_object.get_symbol(name)
        if not symbol:
            raise ValueError(f"missing negative control {name}")
        # Empty slices are valid in both controls. Blanket rejection is a failed test.
        check(project, symbol.rebased_addr, 0, 1)
        try:
            check(project, symbol.rebased_addr, 1, 1)
        except (ValueError, RuntimeError) as error:
            observed = str(error)
            if not observed.startswith(diagnostic):
                raise
            result[name] = observed
        else:
            raise AssertionError(f"checker accepted {name} at length 1")
    return {"architecture": project.arch.name, "object_sha256": hashlib.sha256(elf.read_bytes()).hexdigest(),
            "length_zero_positive_controls": len(controls), "rejections": result}


if __name__ == "__main__":
    print(json.dumps(verify(Path(sys.argv[1]))))
