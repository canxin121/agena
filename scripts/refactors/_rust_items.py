#!/usr/bin/env python3
"""Top-level Rust item inventory built on Agena's conservative Rust lexer."""

from __future__ import annotations

import dataclasses
import importlib.util
import pathlib
import re
import sys
from collections import Counter
from collections.abc import Sequence
from typing import Any

from _common import RefactorError


@dataclasses.dataclass(frozen=True)
class RustItem:
    selector: str
    kind: str
    name: str
    header: str
    start: int
    core_start: int
    end: int
    line: int


def _load_architecture_module() -> Any:
    path = pathlib.Path(__file__).resolve().parents[1] / "rust-architecture-report.py"
    name = "agena_rust_architecture_report_for_refactors"
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RefactorError(f"failed to load Rust lexer: {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ARCH = _load_architecture_module()
BLOCK_KINDS = {"enum", "fn", "impl", "macro", "mod", "trait", "union"}
SEMICOLON_KINDS = {"const", "static", "type", "use"}
NAMED_KINDS = {"const", "enum", "fn", "mod", "static", "struct", "trait", "type", "union"}
MOVABLE_KINDS = NAMED_KINDS | {"impl", "macro"}


def _depths(tokens: Sequence[Any]) -> list[int]:
    depths: list[int] = []
    depth = 0
    for token in tokens:
        depths.append(depth)
        if token.text in ("(", "[", "{"):
            depth += 1
        elif token.text in (")", "]", "}"):
            depth = max(0, depth - 1)
    return depths


def _next_ident(tokens: Sequence[Any], index: int) -> str | None:
    for token in tokens[index + 1 :]:
        if token.kind == "ident":
            return token.text.removeprefix("r#")
        if token.text in (";", "{"):
            return None
    return None


def _normalize_header(source: str, start: int, end: int) -> str:
    return re.sub(r"\s+", " ", source[start:end].strip())


def _inner_header_end(source: str) -> int:
    """Preserve shebang, inner attributes and inner docs at the start of a file."""
    cursor = 0
    length = len(source)
    if source.startswith("#!") and not source.startswith("#!["):
        newline = source.find("\n")
        cursor = length if newline < 0 else newline + 1

    while cursor < length:
        whitespace = re.match(r"\s*", source[cursor:])
        assert whitespace is not None
        candidate = cursor + whitespace.end()
        if source.startswith("//!", candidate):
            newline = source.find("\n", candidate)
            cursor = length if newline < 0 else newline + 1
            continue
        if source.startswith("/*!", candidate):
            end = source.find("*/", candidate + 3)
            if end < 0:
                return cursor
            cursor = end + 2
            continue
        if source.startswith("#![", candidate):
            depth = 0
            index = candidate + 2
            while index < length:
                if source[index] == "[":
                    depth += 1
                elif source[index] == "]":
                    depth -= 1
                    if depth == 0:
                        cursor = index + 1
                        break
                index += 1
            else:
                return cursor
            continue
        break
    return cursor


def _item_end(kind: str, index: int, tokens: Sequence[Any], depths: Sequence[int], pairs: dict[int, int]) -> int:
    for cursor in range(index + 1, len(tokens)):
        if depths[cursor] != 0:
            continue
        token = tokens[cursor]
        if token.text == ";":
            return token.end
        if token.text == "{":
            close = pairs.get(cursor)
            if close is None:
                raise RefactorError(f"unclosed Rust item beginning at byte {tokens[index].start}")
            if kind in BLOCK_KINDS or kind in {"extern", "struct"}:
                return tokens[close].end
    raise RefactorError(f"could not find end of top-level {kind} item at byte {tokens[index].start}")


def _macro_invocation_end(index: int, tokens: Sequence[Any], depths: Sequence[int], pairs: dict[int, int]) -> int | None:
    if index + 2 >= len(tokens) or tokens[index + 1].text != "!":
        return None
    opener = index + 2
    if tokens[opener].text not in ("(", "[", "{"):
        return None
    close = pairs.get(opener)
    if close is None:
        return None
    end = tokens[close].end
    if close + 1 < len(tokens) and depths[close + 1] == 0 and tokens[close + 1].text == ";":
        end = tokens[close + 1].end
    return end


def inventory(source: str) -> list[RustItem]:
    tokens, _comments, lex_warnings = ARCH.lex_rust(source)
    pairs, pair_warnings = ARCH.delimiter_pairs(tokens)
    warnings = lex_warnings + pair_warnings
    if warnings:
        raise RefactorError("Rust lexer warnings: " + "; ".join(warnings))
    depths = _depths(tokens)
    raw: list[tuple[str, str, str, int, int]] = []
    occupied_until = -1

    for index, token in enumerate(tokens):
        if token.start < occupied_until or depths[index] != 0 or token.kind != "ident":
            continue
        text = token.text.removeprefix("r#")
        kind: str | None = None
        name = ""
        end: int | None = None

        if text == "macro_rules" and index + 2 < len(tokens) and tokens[index + 1].text == "!":
            kind = "macro"
            name = tokens[index + 2].text.removeprefix("r#")
            end = _item_end(kind, index, tokens, depths, pairs)
        elif text in NAMED_KINDS | {"impl", "use"}:
            kind = text
            name = _next_ident(tokens, index) or text
            end = _item_end(kind, index, tokens, depths, pairs)
        elif text == "extern":
            # Only treat an extern block/crate as its own item. `extern fn` is
            # discovered at the later top-level `fn` token.
            lookahead = [candidate.text for candidate in tokens[index + 1 : index + 5]]
            if "crate" in lookahead or "{" in lookahead:
                kind = "extern"
                name = "crate" if "crate" in lookahead else "block"
                end = _item_end(kind, index, tokens, depths, pairs)
        else:
            macro_end = _macro_invocation_end(index, tokens, depths, pairs)
            if macro_end is not None:
                kind = "macro"
                name = text
                end = macro_end

        if kind is None or end is None:
            continue
        header_end = next(
            (candidate.start for candidate in tokens[index + 1 :] if candidate.text in ("{", ";")),
            end,
        )
        header = _normalize_header(source, token.start, header_end)
        raw.append((kind, name, header, token.start, end))
        occupied_until = end

    raw.sort(key=lambda value: value[3])
    header_end = _inner_header_end(source)
    previous_end = header_end
    bases: list[str] = []
    for kind, name, _header, _core_start, _end in raw:
        if kind == "impl":
            bases.append("impl")
        elif kind == "macro":
            bases.append(f"macro:{name}")
        else:
            bases.append(f"{kind}:{name}")
    totals = Counter(bases)
    seen: Counter[str] = Counter()
    result: list[RustItem] = []
    impl_index = 0

    for (kind, name, header, core_start, end), base in zip(raw, bases, strict=True):
        if kind == "impl":
            impl_index += 1
            selector = f"impl@{impl_index}"
        else:
            seen[base] += 1
            selector = base if totals[base] == 1 else f"{base}#{seen[base]}"
        start = previous_end
        previous_end = end
        result.append(
            RustItem(
                selector=selector,
                kind=kind,
                name=name,
                header=header,
                start=start,
                core_start=core_start,
                end=end,
                line=source.count("\n", 0, core_start) + 1,
            )
        )
    return result


def movable_items(source: str) -> list[RustItem]:
    return [item for item in inventory(source) if item.kind in MOVABLE_KINDS]
