#!/usr/bin/env python3
"""Apply exact, count-asserted text replacements to an explicit file list.

All replacements are simulated in memory first. The command is dry-run by
default and performs no writes if any expected count or path check fails.
"""

from __future__ import annotations

import argparse
import pathlib
import sys
from collections.abc import Sequence

from _common import (
    RefactorError,
    atomic_write_many,
    load_manifest,
    relative_display,
    render_diffs,
    require_list,
    require_string,
    resolve_root,
    resolve_within,
)


def _non_negative_int(value: object, field: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise RefactorError(f"{field} must be a non-negative integer")
    return value


def run(args: argparse.Namespace) -> int:
    root = resolve_root(args.root)
    manifest = load_manifest(pathlib.Path(args.manifest).resolve())
    replacements = require_list(manifest.get("replacements"), "replacements")
    if not replacements:
        raise RefactorError("replacements must not be empty")

    contents: dict[pathlib.Path, str] = {}
    summaries: list[str] = []
    for index, raw_replacement in enumerate(replacements):
        field = f"replacements[{index}]"
        if not isinstance(raw_replacement, dict):
            raise RefactorError(f"{field} must be an object")
        old = require_string(raw_replacement.get("old"), f"{field}.old")
        new = require_string(raw_replacement.get("new"), f"{field}.new", allow_empty=True)
        if old == new:
            raise RefactorError(f"{field}.old and {field}.new must differ")
        expected = _non_negative_int(raw_replacement.get("expected"), f"{field}.expected")
        expected_new_before_raw = raw_replacement.get("expected_new_before")
        expected_new_before = (
            None
            if expected_new_before_raw is None
            else _non_negative_int(expected_new_before_raw, f"{field}.expected_new_before")
        )
        raw_files = require_list(raw_replacement.get("files"), f"{field}.files")
        if not raw_files:
            raise RefactorError(f"{field}.files must not be empty")
        paths: list[pathlib.Path] = []
        seen_paths: set[pathlib.Path] = set()
        for file_index, raw_file in enumerate(raw_files):
            path = resolve_within(
                root,
                require_string(raw_file, f"{field}.files[{file_index}]"),
                f"{field}.files[{file_index}]",
            )
            if not path.is_file():
                raise RefactorError(f"replacement input is not a file: {relative_display(path, root)}")
            if path in seen_paths:
                raise RefactorError(
                    f"{field}.files contains a duplicate path: {relative_display(path, root)}"
                )
            seen_paths.add(path)
            if path not in contents:
                contents[path] = path.read_text(encoding="utf-8")
            paths.append(path)

        old_count = sum(contents[path].count(old) for path in paths)
        if old_count != expected:
            raise RefactorError(
                f"{field} expected {expected} occurrence(s) of {old!r}, found {old_count}"
            )
        new_before = sum(contents[path].count(new) for path in paths) if new else 0
        if expected_new_before is not None and new_before != expected_new_before:
            raise RefactorError(
                f"{field} expected {expected_new_before} pre-existing occurrence(s) of "
                f"{new!r}, found {new_before}"
            )
        for path in paths:
            contents[path] = contents[path].replace(old, new)
        summaries.append(
            f"{field}: {old!r} -> {new!r} in {len(paths)} file(s), {old_count} replacement(s)"
        )

    changes = {
        path: content
        for path, content in contents.items()
        if path.read_text(encoding="utf-8") != content
    }
    if args.show_diff:
        print(render_diffs(changes, root), end="")
    for summary in summaries:
        print(summary)
    print(f"validated {len(replacements)} replacement group(s), {len(changes)} changed file(s)")
    if not args.apply:
        print("dry-run only; pass --apply to write the validated changes")
        return 0
    atomic_write_many(changes)
    print("applied asserted replacements")
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, help="JSON replacement manifest")
    parser.add_argument("--root", type=pathlib.Path, help="refactor root; defaults to cwd")
    parser.add_argument("--apply", action="store_true", help="write files after validation")
    parser.add_argument("--show-diff", action="store_true", help="print the proposed unified diff")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        return run(parse_args(sys.argv[1:] if argv is None else argv))
    except RefactorError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
