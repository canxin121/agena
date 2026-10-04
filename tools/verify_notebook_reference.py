#!/usr/bin/env python3
"""Verify the native validator's corpus and exported edits against nbformat.

Run with: uv run --no-project --with nbformat==5.11.1 python
          tools/verify_notebook_reference.py [--outputs /tmp/edited-notebooks.json]
No Agena service, tool invocation or network request is made by this script.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import warnings
from pathlib import Path

import nbformat


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outputs", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    schemas = root / "crates/agena-bundled-plugins/src/plugins/provided/notebook/schemas"
    upstream = json.loads((schemas / "UPSTREAM.json").read_text())
    assert nbformat.__version__ == upstream["version"], "Use the recorded nbformat reference version"
    for item in upstream["files"]:
        contents = (schemas / item["file"]).read_bytes()
        assert hashlib.sha256(contents).hexdigest() == item["sha256"]
        installed = Path(nbformat.__file__).parent / "v4" / item["file"]
        assert contents == installed.read_bytes(), f"Vendored schema differs: {item['file']}"
    corpus = json.loads((schemas / "reference-corpus.json").read_text())
    for case in corpus["cases"]:
        with warnings.catch_warnings(record=True) as observed:
            warnings.simplefilter("always")
            try:
                nbformat.validate(copy.deepcopy(case["notebook"]))
                valid = True
            except nbformat.ValidationError:
                valid = False
        warning_names = [warning.category.__name__ for warning in observed]
        assert warning_names == case.get("reference_warnings", []), f"Reference warning drift: {case['name']}"
        assert valid == case["valid"], f"Reference drift: {case['name']}"
    outputs = [] if args.outputs is None else json.loads(args.outputs.read_text())
    for notebook in outputs:
        nbformat.validate(notebook)
    print(f"nbformat {nbformat.__version__}: {len(upstream['files'])} schemas, "
          f"{len(corpus['cases'])} reference cases, {len(outputs)} edited notebooks passed")


if __name__ == "__main__":
    main()
