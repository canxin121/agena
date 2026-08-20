#!/usr/bin/env python3
"""Compile C-SKY C sources, splitting oversized Tree-sitter lexers on demand."""

from __future__ import annotations

import os
import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path


_TRANSFORMER_PATH = Path(__file__).with_name("split-tree-sitter-parser.py")
_TRANSFORMER_SPEC = importlib.util.spec_from_file_location(
    "agena_csky_tree_sitter_transformer", _TRANSFORMER_PATH
)
if _TRANSFORMER_SPEC is None or _TRANSFORMER_SPEC.loader is None:
    raise ImportError(f"cannot load {_TRANSFORMER_PATH}")
_TRANSFORMER = importlib.util.module_from_spec(_TRANSFORMER_SPEC)
_TRANSFORMER_SPEC.loader.exec_module(_TRANSFORMER)
transform = _TRANSFORMER.transform


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        raise SystemExit("usage: csky-cc-wrapper.py COMPILER [ARGS ...]")

    compiler = argv[1]
    args = argv[2:]
    parser_index = next(
        (
            index
            for index, arg in enumerate(args)
            if arg.endswith("/parser.c") or arg.endswith("\\parser.c")
        ),
        None,
    )
    if parser_index is None:
        return subprocess.call([compiler, *args])

    source = Path(args[parser_index])
    try:
        original = source.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return subprocess.call([compiler, *args])
    transformed = transform(original)
    if transformed is None:
        return subprocess.call([compiler, *args])

    runner_temp = os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()
    with tempfile.TemporaryDirectory(prefix="agena-csky-parser-", dir=runner_temp) as directory:
        generated = Path(directory) / "parser.c"
        generated.write_text(transformed, encoding="utf-8")
        args[parser_index] = str(generated)
        return subprocess.call([compiler, *args])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
