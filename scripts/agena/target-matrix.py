#!/usr/bin/env python3
"""Render checked-in universal target metadata for GitHub Actions."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("group", choices=("hosted-cross", "native", "portable", "portable-build-std"))
    parser.add_argument("--compact", action="store_true")
    args = parser.parse_args()

    payload = json.loads(MANIFEST.read_text(encoding="utf-8"))
    if args.group == "hosted-cross":
        key = "hosted_cross"
    elif args.group == "native":
        key = "native"
    elif args.group == "portable":
        key = "portable"
    else:
        key = "portable_build_std"
    matrix = {"include": payload[key]}
    print(json.dumps(matrix, separators=(",", ":") if args.compact else None))


if __name__ == "__main__":
    main()
