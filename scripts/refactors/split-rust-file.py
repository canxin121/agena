#!/usr/bin/env python3
"""Inventory or split top-level Rust items without retyping their bodies.

The command is dry-run by default. It reuses the conservative lexer from
``scripts/rust-architecture-report.py`` and writes only after every selector,
destination, and manifest invariant has been validated.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
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
from _rust_items import RustItem, movable_items


def inventory_command(args: argparse.Namespace) -> int:
    root = resolve_root(args.root)
    source_path = resolve_within(root, args.source, "--source")
    if not source_path.is_file():
        raise RefactorError(f"Rust source does not exist: {relative_display(source_path, root)}")
    source = source_path.read_text(encoding="utf-8")
    items = movable_items(source)
    if args.json:
        print(
            json.dumps(
                [dataclasses.asdict(item) for item in items],
                ensure_ascii=False,
                indent=2,
            )
        )
    else:
        for item in items:
            print(f"{item.selector}\tline={item.line}\t{item.header}")
        print(f"{len(items)} movable top-level item(s)", file=sys.stderr)
    return 0


def _destination_content(
    source: str,
    selected: Sequence[RustItem],
    header: str,
) -> str:
    parts: list[str] = []
    if header.strip():
        parts.append(header.rstrip())
    for item in sorted(selected, key=lambda value: value.core_start):
        text = source[item.start : item.end].strip()
        if not text:
            raise RefactorError(f"selector produced empty text: {item.selector}")
        parts.append(text)
    return "\n\n".join(parts).rstrip() + "\n"


def _remove_spans(source: str, selected: Sequence[RustItem]) -> str:
    result = source
    for item in sorted(selected, key=lambda value: value.start, reverse=True):
        result = result[: item.start] + result[item.end :]
    return result


def split_command(args: argparse.Namespace) -> int:
    root = resolve_root(args.root)
    manifest_path = pathlib.Path(args.manifest).resolve()
    manifest = load_manifest(manifest_path)
    source_path = resolve_within(
        root,
        require_string(manifest.get("source"), "source"),
        "source",
    )
    if not source_path.is_file():
        raise RefactorError(f"Rust source does not exist: {relative_display(source_path, root)}")
    source = source_path.read_text(encoding="utf-8")
    items = movable_items(source)
    by_selector = {item.selector: item for item in items}
    destinations = require_list(manifest.get("destinations"), "destinations")
    if not destinations:
        raise RefactorError("destinations must not be empty")

    requested: set[str] = set()
    selected_all: list[RustItem] = []
    changes: dict[pathlib.Path, str] = {}
    summaries: list[str] = []

    for index, raw_destination in enumerate(destinations):
        field = f"destinations[{index}]"
        if not isinstance(raw_destination, dict):
            raise RefactorError(f"{field} must be an object")
        destination = resolve_within(
            root,
            require_string(raw_destination.get("path"), f"{field}.path"),
            f"{field}.path",
        )
        if destination == source_path:
            raise RefactorError(f"{field}.path must not equal source")
        if destination in changes:
            raise RefactorError(f"duplicate destination path: {relative_display(destination, root)}")
        if destination.exists():
            raise RefactorError(
                f"destination already exists; refusing to overwrite: {relative_display(destination, root)}"
            )
        selectors = require_list(raw_destination.get("items"), f"{field}.items")
        if not selectors:
            raise RefactorError(f"{field}.items must not be empty")
        selected: list[RustItem] = []
        for selector_index, raw_selector in enumerate(selectors):
            selector = require_string(raw_selector, f"{field}.items[{selector_index}]")
            if selector in requested:
                raise RefactorError(f"selector requested by more than one destination: {selector}")
            item = by_selector.get(selector)
            if item is None:
                available = ", ".join(sorted(by_selector))
                raise RefactorError(f"unknown selector {selector!r}; available: {available}")
            requested.add(selector)
            selected.append(item)
            selected_all.append(item)
        header = require_string(
            raw_destination.get("header", ""),
            f"{field}.header",
            allow_empty=True,
        )
        changes[destination] = _destination_content(source, selected, header)
        summaries.append(
            f"{relative_display(destination, root)} <- "
            + ", ".join(item.selector for item in selected)
        )

    # Item spans are disjoint by construction, but assert it before touching files.
    ordered = sorted(selected_all, key=lambda value: value.start)
    for left, right in zip(ordered, ordered[1:]):
        if left.end > right.start:
            raise RefactorError(f"overlapping item spans: {left.selector} and {right.selector}")

    changes[source_path] = _remove_spans(source, selected_all)
    changed = [
        path
        for path, content in changes.items()
        if not path.exists() or path.read_text(encoding="utf-8") != content
    ]
    if args.show_diff:
        print(render_diffs(changes, root), end="")
    for summary in summaries:
        print(summary)
    print(
        f"validated {len(selected_all)} item(s), {len(destinations)} destination(s), "
        f"{len(changed)} changed file(s)"
    )
    if not args.apply:
        print("dry-run only; pass --apply to write the validated changes")
        return 0
    atomic_write_many(changes)
    print("applied Rust item split")
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=pathlib.Path,
        help="refactor root; defaults to the current directory",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    inventory_parser = subparsers.add_parser("list", help="list movable top-level Rust items")
    inventory_parser.add_argument("--source", required=True, help="Rust file relative to root")
    inventory_parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    inventory_parser.set_defaults(handler=inventory_command)

    split_parser = subparsers.add_parser("split", help="validate or apply a split manifest")
    split_parser.add_argument("--manifest", required=True, help="JSON split manifest")
    split_parser.add_argument("--apply", action="store_true", help="write files after validation")
    split_parser.add_argument("--show-diff", action="store_true", help="print the proposed unified diff")
    split_parser.set_defaults(handler=split_command)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(sys.argv[1:] if argv is None else argv)
        return args.handler(args)
    except RefactorError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
