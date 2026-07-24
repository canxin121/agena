#!/usr/bin/env python3
"""Check path, text, and source-size invariants from a JSON manifest."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from collections.abc import Sequence

from _common import (
    RefactorError,
    find_matching_files,
    load_manifest,
    relative_display,
    require_list,
    require_string,
    resolve_root,
    resolve_within,
)


def _string_list(value: object, field: str, *, default: list[str] | None = None) -> list[str]:
    if value is None and default is not None:
        return default
    raw = require_list(value, field)
    return [require_string(item, f"{field}[{index}]") for index, item in enumerate(raw)]


def _optional_int(value: object, field: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise RefactorError(f"{field} must be a non-negative integer")
    return value


def _line_hits(text: str, pattern: str, regex: bool) -> list[int]:
    expression = re.compile(pattern) if regex else re.compile(re.escape(pattern))
    return [text.count("\n", 0, match.start()) + 1 for match in expression.finditer(text)]


def run(args: argparse.Namespace) -> int:
    root = resolve_root(args.root)
    manifest = load_manifest(pathlib.Path(args.manifest).resolve())
    failures: list[str] = []
    checks = 0

    for index, raw in enumerate(_string_list(manifest.get("must_exist", []), "must_exist")):
        checks += 1
        path = resolve_within(root, raw, f"must_exist[{index}]")
        if not path.exists():
            failures.append(f"required path does not exist: {raw}")

    for index, raw in enumerate(_string_list(manifest.get("must_not_exist", []), "must_not_exist")):
        checks += 1
        path = resolve_within(root, raw, f"must_not_exist[{index}]")
        if path.exists():
            failures.append(f"forbidden path exists: {raw}")

    text_rules = require_list(manifest.get("text_rules", []), "text_rules")
    for index, raw_rule in enumerate(text_rules):
        field = f"text_rules[{index}]"
        if not isinstance(raw_rule, dict):
            raise RefactorError(f"{field} must be an object")
        name = require_string(raw_rule.get("name", field), f"{field}.name")
        roots = _string_list(raw_rule.get("roots"), f"{field}.roots")
        includes = _string_list(raw_rule.get("include"), f"{field}.include", default=["**/*"])
        excludes = _string_list(raw_rule.get("exclude"), f"{field}.exclude", default=[])
        pattern = require_string(raw_rule.get("pattern"), f"{field}.pattern")
        regex = raw_rule.get("regex", False)
        if not isinstance(regex, bool):
            raise RefactorError(f"{field}.regex must be a boolean")
        expected = _optional_int(raw_rule.get("expected"), f"{field}.expected")
        minimum = _optional_int(raw_rule.get("min"), f"{field}.min")
        maximum = _optional_int(raw_rule.get("max"), f"{field}.max")
        if expected is None and minimum is None and maximum is None:
            raise RefactorError(f"{field} must define expected, min, or max")
        files = find_matching_files(root, roots, includes, excludes)
        hits: list[tuple[pathlib.Path, int]] = []
        total = 0
        for path in files:
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError as error:
                raise RefactorError(f"text rule {name!r} matched non-UTF-8 file: {path}") from error
            lines = _line_hits(text, pattern, regex)
            total += len(lines)
            hits.extend((path, line) for line in lines)
        checks += 1
        valid = True
        if expected is not None and total != expected:
            valid = False
        if minimum is not None and total < minimum:
            valid = False
        if maximum is not None and total > maximum:
            valid = False
        if not valid:
            constraint = f"expected={expected}, min={minimum}, max={maximum}"
            locations = ", ".join(
                f"{relative_display(path, root)}:{line}" for path, line in hits[:12]
            )
            if len(hits) > 12:
                locations += f", ... {len(hits) - 12} more"
            failures.append(
                f"text rule {name!r} found {total} occurrence(s) ({constraint})"
                + (f" at {locations}" if locations else "")
            )

    line_rules = require_list(manifest.get("line_rules", []), "line_rules")
    for index, raw_rule in enumerate(line_rules):
        field = f"line_rules[{index}]"
        if not isinstance(raw_rule, dict):
            raise RefactorError(f"{field} must be an object")
        name = require_string(raw_rule.get("name", field), f"{field}.name")
        roots = _string_list(raw_rule.get("roots"), f"{field}.roots")
        includes = _string_list(raw_rule.get("include"), f"{field}.include", default=["**/*.rs"])
        excludes = _string_list(raw_rule.get("exclude"), f"{field}.exclude", default=[])
        maximum = _optional_int(raw_rule.get("max_lines"), f"{field}.max_lines")
        if maximum is None:
            raise RefactorError(f"{field}.max_lines is required")
        files = find_matching_files(root, roots, includes, excludes)
        oversized: list[tuple[pathlib.Path, int]] = []
        for path in files:
            text = path.read_text(encoding="utf-8")
            lines = text.count("\n") + (0 if not text or text.endswith("\n") else 1)
            if lines > maximum:
                oversized.append((path, lines))
        checks += 1
        if oversized:
            rendered = ", ".join(
                f"{relative_display(path, root)}={lines}" for path, lines in oversized
            )
            failures.append(f"line rule {name!r} exceeds {maximum}: {rendered}")

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"{len(failures)} invariant failure(s) across {checks} check(s)", file=sys.stderr)
        return 1
    print(f"all {checks} refactor invariant check(s) passed")
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, help="JSON invariant manifest")
    parser.add_argument("--root", type=pathlib.Path, help="refactor root; defaults to cwd")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        return run(parse_args(sys.argv[1:] if argv is None else argv))
    except (RefactorError, re.error) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
