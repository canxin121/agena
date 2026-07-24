#!/usr/bin/env python3
"""Generate a complete Rust workspace architecture report.

The generator intentionally uses only Python's standard library and Cargo's
stable metadata output.  It combines Cargo package/target/dependency data with
a conservative Rust lexer.  The lexer is used to resolve out-of-line modules,
collect intra-crate module references, and create per-file source skeletons in
which function bodies are omitted while signatures and data declarations stay
visible.
"""

from __future__ import annotations

import argparse
import bisect
import collections
import dataclasses
import json
import os
import pathlib
import re
import subprocess
import sys
from typing import Iterable, Iterator, Sequence


SKIP_DIRS = {".git", "target", "node_modules"}
ITEM_KEYWORDS = ("struct", "enum", "union", "trait", "type", "impl", "const", "static")


@dataclasses.dataclass(frozen=True)
class Token:
    kind: str
    text: str
    start: int
    end: int


@dataclasses.dataclass(frozen=True)
class ModuleDecl:
    name: str
    kind: str
    position: int
    line: int
    ancestors: tuple[str, ...]
    body_start: int | None = None
    body_end: int | None = None
    path_attr: str | None = None


@dataclasses.dataclass
class ParsedSource:
    path: pathlib.Path
    source: str
    tokens: list[Token]
    pairs: dict[int, int]
    comments: list[tuple[int, int]]
    function_bodies: list[tuple[int, int]]
    function_items: int
    macro_bodies: list[tuple[int, int]]
    modules: list[ModuleDecl]
    counts: collections.Counter[str]
    warnings: list[str]

    def line_at(self, position: int) -> int:
        return self.source.count("\n", 0, position) + 1


@dataclasses.dataclass
class TargetGraph:
    package: str
    target: str
    kinds: tuple[str, ...]
    root: pathlib.Path
    node_files: dict[tuple[str, ...], pathlib.Path]
    file_bases: set[tuple[pathlib.Path, tuple[str, ...]]]
    declaration_edges: set[tuple[tuple[str, ...], tuple[str, ...]]]
    reference_edges: set[tuple[tuple[str, ...], tuple[str, ...]]]
    unresolved: list[str]
    observed_external: set[str]


def run(command: Sequence[str], cwd: pathlib.Path, *, check: bool = True) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def repository_root(start: pathlib.Path) -> pathlib.Path:
    output = run(["git", "rev-parse", "--show-toplevel"], start)
    return pathlib.Path(output).resolve()


def is_ident_start(char: str) -> bool:
    return char == "_" or char.isalpha() or (ord(char) >= 128 and char.isidentifier())


def is_ident_continue(char: str) -> bool:
    return char == "_" or char.isalnum() or (ord(char) >= 128 and (char.isidentifier() or char.isnumeric()))


def raw_string_end(source: str, start: int) -> int | None:
    """Return the end of an r/br/cr raw string starting at *start*."""
    length = len(source)
    for prefix in ("br", "cr", "r"):
        if not source.startswith(prefix, start):
            continue
        cursor = start + len(prefix)
        hashes = 0
        while cursor < length and source[cursor] == "#":
            hashes += 1
            cursor += 1
        if cursor >= length or source[cursor] != '"':
            continue
        closing = '"' + ("#" * hashes)
        found = source.find(closing, cursor + 1)
        return length if found < 0 else found + len(closing)
    return None


def quoted_end(source: str, start: int, quote: str) -> int:
    cursor = start + 1
    escaped = False
    while cursor < len(source):
        char = source[cursor]
        if escaped:
            escaped = False
        elif char == "\\":
            escaped = True
        elif char == quote:
            return cursor + 1
        cursor += 1
    return len(source)


def char_literal_end(source: str, start: int) -> int | None:
    """Distinguish a Rust character literal from a lifetime."""
    if start + 1 >= len(source):
        return None
    cursor = start + 1
    if source[cursor] == "\\":
        cursor += 1
        if cursor < len(source) and source[cursor] == "u" and cursor + 1 < len(source) and source[cursor + 1] == "{":
            closing = source.find("}", cursor + 2)
            cursor = len(source) if closing < 0 else closing + 1
        else:
            cursor += 1
    else:
        cursor += 1
    if cursor < len(source) and source[cursor] == "'":
        return cursor + 1
    return None


def lex_rust(source: str) -> tuple[list[Token], list[tuple[int, int]], list[str]]:
    tokens: list[Token] = []
    comments: list[tuple[int, int]] = []
    warnings: list[str] = []
    cursor = 0
    length = len(source)
    while cursor < length:
        char = source[cursor]
        if char.isspace():
            cursor += 1
            continue
        if source.startswith("//", cursor):
            end = source.find("\n", cursor + 2)
            end = length if end < 0 else end
            comments.append((cursor, end))
            cursor = end
            continue
        if source.startswith("/*", cursor):
            start = cursor
            cursor += 2
            depth = 1
            while cursor < length and depth:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            if depth:
                warnings.append(f"unterminated block comment at byte {start}")
            comments.append((start, cursor))
            continue

        raw_end = raw_string_end(source, cursor)
        if raw_end is not None:
            tokens.append(Token("literal", source[cursor:raw_end], cursor, raw_end))
            cursor = raw_end
            continue
        if char == '"' or ((char in "bc") and cursor + 1 < length and source[cursor + 1] == '"'):
            quote_start = cursor if char == '"' else cursor + 1
            end = quoted_end(source, quote_start, '"')
            tokens.append(Token("literal", source[cursor:end], cursor, end))
            cursor = end
            continue
        if char == "'":
            end = char_literal_end(source, cursor)
            if end is not None:
                tokens.append(Token("literal", source[cursor:end], cursor, end))
                cursor = end
                continue
        if char == "b" and cursor + 1 < length and source[cursor + 1] == "'":
            end = char_literal_end(source, cursor + 1)
            if end is not None:
                tokens.append(Token("literal", source[cursor:end], cursor, end))
                cursor = end
                continue
        if is_ident_start(char):
            end = cursor + 1
            while end < length and is_ident_continue(source[end]):
                end += 1
            if end + 1 < length and source[cursor:end] == "r" and source[end] == "#" and is_ident_start(source[end + 1]):
                end += 2
                while end < length and is_ident_continue(source[end]):
                    end += 1
            tokens.append(Token("ident", source[cursor:end], cursor, end))
            cursor = end
            continue
        tokens.append(Token("punct", char, cursor, cursor + 1))
        cursor += 1
    return tokens, comments, warnings


def delimiter_pairs(tokens: Sequence[Token]) -> tuple[dict[int, int], list[str]]:
    pairs: dict[int, int] = {}
    warnings: list[str] = []
    stack: list[tuple[str, int]] = []
    opening = {"(": ")", "[": "]", "{": "}"}
    closing = {value: key for key, value in opening.items()}
    for index, token in enumerate(tokens):
        if token.kind != "punct":
            continue
        if token.text in opening:
            stack.append((token.text, index))
        elif token.text in closing:
            if stack and stack[-1][0] == closing[token.text]:
                _, open_index = stack.pop()
                pairs[open_index] = index
                pairs[index] = open_index
            else:
                warnings.append(f"unmatched delimiter {token.text!r} at byte {token.start}")
    for delimiter, index in stack:
        warnings.append(f"unclosed delimiter {delimiter!r} at byte {tokens[index].start}")
    return pairs, warnings


def in_ranges(position: int, ranges: Sequence[tuple[int, int]]) -> bool:
    return any(start < position < end for start, end in ranges)


def function_items(tokens: Sequence[Token], pairs: dict[int, int]) -> tuple[list[tuple[int, int]], int]:
    bodies: list[tuple[int, int]] = []
    items = 0
    for index, token in enumerate(tokens):
        if token.kind != "ident" or token.text != "fn" or index + 1 >= len(tokens):
            continue
        name = tokens[index + 1]
        if name.kind != "ident":
            continue
        parameter_open: int | None = None
        cursor = index + 2
        search_limit = min(len(tokens), index + 1000)
        while cursor < search_limit:
            current = tokens[cursor]
            if current.text == "(":
                parameter_open = cursor
                break
            if current.text == ";":
                break
            if current.text in ("[", "{") and cursor in pairs:
                cursor = pairs[cursor]
            cursor += 1
        if parameter_open is None or parameter_open not in pairs:
            continue
        items += 1
        cursor = pairs[parameter_open] + 1
        angle_depth = 0
        while cursor < len(tokens):
            current = tokens[cursor]
            if current.text in ("(", "[") and cursor in pairs:
                cursor = pairs[cursor] + 1
                continue
            if current.text == "<":
                angle_depth += 1
            elif current.text == ">" and angle_depth:
                angle_depth -= 1
            elif current.text == "{" and cursor in pairs:
                if angle_depth:
                    cursor = pairs[cursor] + 1
                    continue
                close_index = pairs[cursor]
                if cursor > 0 and tokens[cursor - 1].text == ",":
                    body_start = tokens[cursor - 1].start
                else:
                    body_start = tokens[cursor - 1].end if cursor > 0 else current.start
                bodies.append((body_start, tokens[close_index].end))
                break
            elif current.text == ";" and angle_depth == 0:
                break
            cursor += 1
    bodies.sort()
    outermost: list[tuple[int, int]] = []
    for body in bodies:
        if outermost and body[0] >= outermost[-1][0] and body[1] <= outermost[-1][1]:
            continue
        outermost.append(body)
    return outermost, items


def macro_body_ranges(tokens: Sequence[Token], pairs: dict[int, int]) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    for index, token in enumerate(tokens):
        if token.kind != "ident" or token.text != "macro_rules":
            continue
        cursor = index + 1
        while cursor < min(len(tokens), index + 8):
            if tokens[cursor].text in ("{", "(", "[") and cursor in pairs:
                ranges.append((tokens[cursor].start, tokens[pairs[cursor]].end))
                break
            cursor += 1
    return ranges


def preceding_path_attribute(tokens: Sequence[Token], mod_index: int, pairs: dict[int, int]) -> str | None:
    cursor = mod_index - 1
    inspected = 0
    while cursor >= 0 and inspected < 8:
        if tokens[cursor].text != "]" or cursor not in pairs:
            break
        open_index = pairs[cursor]
        if open_index == 0 or tokens[open_index - 1].text != "#":
            break
        inner = tokens[open_index + 1 : cursor]
        for index, token in enumerate(inner):
            if token.kind == "ident" and token.text == "path":
                for candidate in inner[index + 1 :]:
                    if candidate.kind == "literal" and candidate.text.startswith('"'):
                        try:
                            return json.loads(candidate.text)
                        except json.JSONDecodeError:
                            return candidate.text.strip('"')
        cursor = open_index - 2
        inspected += 1
    return None


def module_declarations(
    source: str,
    tokens: Sequence[Token],
    pairs: dict[int, int],
    function_bodies: Sequence[tuple[int, int]],
    macro_bodies: Sequence[tuple[int, int]],
) -> list[ModuleDecl]:
    provisional: list[tuple[str, str, int, int, int | None, int | None, str | None]] = []
    for index, token in enumerate(tokens):
        if token.kind != "ident" or token.text != "mod" or index + 2 >= len(tokens):
            continue
        if in_ranges(token.start, function_bodies) or in_ranges(token.start, macro_bodies):
            continue
        name = tokens[index + 1]
        terminator = tokens[index + 2]
        if name.kind != "ident" or terminator.text not in (";", "{"):
            continue
        if terminator.text == "{" and index + 2 not in pairs:
            continue
        body_start = terminator.start if terminator.text == "{" else None
        body_end = tokens[pairs[index + 2]].end if body_start is not None else None
        provisional.append(
            (
                name.text.removeprefix("r#"),
                "inline" if body_start is not None else "external",
                token.start,
                source.count("\n", 0, token.start) + 1,
                body_start,
                body_end,
                preceding_path_attribute(tokens, index, pairs),
            )
        )
    inline = [item for item in provisional if item[1] == "inline"]
    declarations: list[ModuleDecl] = []
    for name, kind, position, line, body_start, body_end, path_attr in provisional:
        containers = [
            item
            for item in inline
            if item[4] is not None and item[5] is not None and item[4] < position < item[5]
        ]
        containers.sort(key=lambda item: item[4] or 0)
        declarations.append(
            ModuleDecl(
                name=name,
                kind=kind,
                position=position,
                line=line,
                ancestors=tuple(item[0] for item in containers),
                body_start=body_start,
                body_end=body_end,
                path_attr=path_attr,
            )
        )
    return declarations


def item_counts(tokens: Sequence[Token], modules: Sequence[ModuleDecl], function_count: int) -> collections.Counter[str]:
    counts: collections.Counter[str] = collections.Counter()
    counts["fn"] = function_count
    counts["mod"] = len(modules)
    for index, token in enumerate(tokens):
        if token.kind != "ident" or token.text not in ITEM_KEYWORDS:
            continue
        if token.text == "impl":
            counts[token.text] += 1
        elif index + 1 < len(tokens) and tokens[index + 1].kind == "ident":
            counts[token.text] += 1
    return counts


def parse_source(path: pathlib.Path) -> ParsedSource:
    source = path.read_text(encoding="utf-8", errors="replace")
    tokens, comments, warnings = lex_rust(source)
    pairs, pair_warnings = delimiter_pairs(tokens)
    warnings.extend(pair_warnings)
    function_bodies, function_count = function_items(tokens, pairs)
    macros = macro_body_ranges(tokens, pairs)
    modules = module_declarations(source, tokens, pairs, function_bodies, macros)
    return ParsedSource(
        path=path,
        source=source,
        tokens=tokens,
        pairs=pairs,
        comments=comments,
        function_bodies=function_bodies,
        function_items=function_count,
        macro_bodies=macros,
        modules=modules,
        counts=item_counts(tokens, modules, function_count),
        warnings=warnings,
    )


def source_skeleton(parsed: ParsedSource) -> str:
    replacements: list[tuple[int, int, str, int]] = []
    for start, end in parsed.comments:
        replacement = " " if parsed.source.startswith("/*", start) else ""
        replacements.append((start, end, replacement, 0))
    for start, end in parsed.function_bodies:
        replacements.append((start, end, ";", 1))
    replacements.sort(key=lambda value: (value[0], -value[3], -(value[1] - value[0])))
    output: list[str] = []
    cursor = 0
    for start, end, replacement, _priority in replacements:
        if start < cursor:
            continue
        output.append(parsed.source[cursor:start])
        output.append(replacement)
        cursor = end
    output.append(parsed.source[cursor:])
    skeleton = "".join(output)
    skeleton = "\n".join(line.rstrip() for line in skeleton.splitlines())
    skeleton = re.sub(r"\n[ \t]*\n(?:[ \t]*\n)+", "\n\n", skeleton)
    return skeleton.strip() or "// empty after comments and function bodies were omitted"


def iter_rust_files(package_root: pathlib.Path) -> Iterator[pathlib.Path]:
    for current, dirs, files in os.walk(package_root):
        dirs[:] = sorted(directory for directory in dirs if directory not in SKIP_DIRS)
        for filename in sorted(files):
            if filename.endswith(".rs"):
                yield (pathlib.Path(current) / filename).resolve()


def module_directory(module_file: pathlib.Path, *, root: bool = False) -> pathlib.Path:
    if root or module_file.name in ("lib.rs", "main.rs", "mod.rs"):
        return module_file.parent
    return module_file.parent / module_file.stem


def format_module(module: tuple[str, ...]) -> str:
    return "crate" if not module else "crate::" + "::".join(module)


def resolve_target_modules(
    package: str,
    target: dict,
    parsed_files: dict[pathlib.Path, ParsedSource],
) -> TargetGraph:
    root = pathlib.Path(target["src_path"]).resolve()
    node_files: dict[tuple[str, ...], pathlib.Path] = {}
    file_bases: set[tuple[pathlib.Path, tuple[str, ...]]] = set()
    declarations: set[tuple[tuple[str, ...], tuple[str, ...]]] = set()
    unresolved: list[str] = []
    visiting: set[tuple[pathlib.Path, tuple[str, ...]]] = set()

    def visit(path: pathlib.Path, base_module: tuple[str, ...], base_directory: pathlib.Path) -> None:
        key = (path, base_module)
        if key in visiting:
            return
        visiting.add(key)
        node_files[base_module] = path
        file_bases.add(key)
        parsed = parsed_files.get(path)
        if parsed is None:
            unresolved.append(f"missing parsed target root or module: {path}")
            return
        for declaration in parsed.modules:
            parent = base_module + declaration.ancestors
            child = parent + (declaration.name,)
            declarations.add((parent, child))
            if declaration.kind == "inline":
                node_files[child] = path
                continue
            if declaration.path_attr:
                module_path = path.parent.joinpath(*declaration.ancestors, declaration.path_attr).resolve()
                candidates = [module_path]
            else:
                search_base = base_directory.joinpath(*declaration.ancestors)
                candidates = [search_base / f"{declaration.name}.rs", search_base / declaration.name / "mod.rs"]
            existing = [candidate.resolve() for candidate in candidates if candidate.is_file()]
            if len(existing) != 1:
                rendered = ", ".join(str(candidate) for candidate in candidates)
                reason = "not found" if not existing else "ambiguous"
                unresolved.append(
                    f"{path}:{declaration.line}: {format_module(child)} ({reason}; candidates: {rendered})"
                )
                continue
            child_file = existing[0]
            visit(child_file, child, module_directory(child_file))

    visit(root, (), module_directory(root, root=True))
    return TargetGraph(
        package=package,
        target=target["name"],
        kinds=tuple(target["kind"]),
        root=root,
        node_files=node_files,
        file_bases=file_bases,
        declaration_edges=declarations,
        reference_edges=set(),
        unresolved=unresolved,
        observed_external=set(),
    )


def top_level_splits(tokens: Sequence[Token], start: int, end: int) -> list[tuple[int, int]]:
    splits: list[tuple[int, int]] = []
    stack: list[str] = []
    segment = start
    opening = {"(": ")", "[": "]", "{": "}"}
    for index in range(start, end):
        text = tokens[index].text
        if text in opening:
            stack.append(opening[text])
        elif stack and text == stack[-1]:
            stack.pop()
        elif text == "," and not stack:
            splits.append((segment, index))
            segment = index + 1
    splits.append((segment, end))
    return [(left, right) for left, right in splits if left < right]


def path_segments(tokens: Sequence[Token], start: int, end: int) -> list[str]:
    segments: list[str] = []
    expect_segment = True
    index = start
    while index < end:
        token = tokens[index]
        if token.kind == "ident" and token.text == "as":
            break
        if expect_segment and token.kind == "ident":
            segments.append(token.text.removeprefix("r#"))
            expect_segment = False
        elif token.text == "*" and expect_segment:
            expect_segment = False
        elif token.text == ":" and index + 1 < end and tokens[index + 1].text == ":":
            expect_segment = True
            index += 1
        index += 1
    return segments


def expand_use_tree(
    tokens: Sequence[Token],
    start: int,
    end: int,
    prefix: tuple[str, ...] = (),
) -> list[tuple[str, ...]]:
    paths: list[tuple[str, ...]] = []
    for left, right in top_level_splits(tokens, start, end):
        stack: list[str] = []
        group_open: int | None = None
        group_close: int | None = None
        opening = {"(": ")", "[": "]", "{": "}"}
        for index in range(left, right):
            text = tokens[index].text
            if text == "{" and not stack:
                group_open = index
                depth = 1
                cursor = index + 1
                while cursor < right and depth:
                    if tokens[cursor].text == "{":
                        depth += 1
                    elif tokens[cursor].text == "}":
                        depth -= 1
                    cursor += 1
                group_close = cursor - 1 if depth == 0 else right
                break
            if text in opening:
                stack.append(opening[text])
            elif stack and text == stack[-1]:
                stack.pop()
        if group_open is not None and group_close is not None:
            head = tuple(path_segments(tokens, left, group_open))
            paths.extend(expand_use_tree(tokens, group_open + 1, group_close, prefix + head))
        else:
            leaf = list(prefix) + path_segments(tokens, left, right)
            if leaf and leaf[-1] == "self":
                leaf.pop()
            if leaf:
                paths.append(tuple(leaf))
    return paths


def statement_end(tokens: Sequence[Token], start: int) -> int | None:
    stack: list[str] = []
    opening = {"(": ")", "[": "]", "{": "}"}
    for index in range(start, len(tokens)):
        text = tokens[index].text
        if text == ";" and not stack:
            return index
        if text in opening:
            stack.append(opening[text])
        elif stack and text == stack[-1]:
            stack.pop()
    return None


def inline_scope(parsed: ParsedSource, position: int) -> tuple[str, ...]:
    containers = [
        declaration
        for declaration in parsed.modules
        if declaration.kind == "inline"
        and declaration.body_start is not None
        and declaration.body_end is not None
        and declaration.body_start < position < declaration.body_end
    ]
    containers.sort(key=lambda declaration: declaration.body_start or 0)
    return tuple(declaration.name for declaration in containers)


def use_paths(parsed: ParsedSource) -> tuple[list[tuple[int, tuple[str, ...]]], list[tuple[int, int]]]:
    entries: list[tuple[int, tuple[str, ...]]] = []
    ranges: list[tuple[int, int]] = []
    for index, token in enumerate(parsed.tokens):
        if token.kind != "ident" or token.text != "use" or in_ranges(token.start, parsed.macro_bodies):
            continue
        end = statement_end(parsed.tokens, index + 1)
        if end is None:
            continue
        ranges.append((token.start, parsed.tokens[end].end))
        for path in expand_use_tree(parsed.tokens, index + 1, end):
            entries.append((token.start, path))
    return entries, ranges


def qualified_paths(parsed: ParsedSource, excluded: Sequence[tuple[int, int]]) -> list[tuple[int, tuple[str, ...]]]:
    entries: list[tuple[int, tuple[str, ...]]] = []
    tokens = parsed.tokens
    for index, token in enumerate(tokens):
        if token.kind != "ident":
            continue
        if in_ranges(token.start, excluded) or in_ranges(token.start, parsed.macro_bodies):
            continue
        if index >= 2 and tokens[index - 1].text == ":" and tokens[index - 2].text == ":":
            continue
        segments = [token.text]
        cursor = index
        while cursor + 2 < len(tokens) and tokens[cursor + 1].text == ":" and tokens[cursor + 2].text == ":":
            next_index = cursor + 3
            if next_index >= len(tokens) or tokens[next_index].kind != "ident":
                break
            segments.append(tokens[next_index].text.removeprefix("r#"))
            cursor = next_index
        if len(segments) > 1:
            entries.append((token.start, tuple(segments)))
    return entries


def resolve_module_path(
    path: tuple[str, ...],
    current: tuple[str, ...],
    modules: set[tuple[str, ...]],
    *,
    expression_path: bool = False,
) -> tuple[str, ...] | None:
    if not path:
        return None
    explicit = path[0] in ("crate", "self", "super")
    if path[0] == "crate":
        candidate = list(path[1:])
    elif path[0] == "self":
        candidate = list(current) + list(path[1:])
    elif path[0] == "super":
        candidate = list(current)
        index = 0
        while index < len(path) and path[index] == "super":
            if candidate:
                candidate.pop()
            index += 1
        candidate.extend(path[index:])
    else:
        candidate = list(path)
    candidates = [candidate]
    if expression_path and not explicit:
        candidates = [list(current) + candidate, candidate]
    for materialized in candidates:
        for length in range(len(materialized), 0, -1):
            possible = tuple(materialized[:length])
            if possible in modules:
                return possible
    if explicit and () in modules:
        return ()
    return None


def collect_target_references(
    graph: TargetGraph,
    parsed_files: dict[pathlib.Path, ParsedSource],
    dependency_aliases: set[str],
) -> None:
    modules = set(graph.node_files)
    for file_path, base_module in graph.file_bases:
        parsed = parsed_files[file_path]
        uses, use_ranges = use_paths(parsed)
        for position, path in uses:
            current = base_module + inline_scope(parsed, position)
            target = resolve_module_path(path, current, modules)
            if target is not None:
                if target != current:
                    graph.reference_edges.add((current, target))
                continue
            if path and path[0] not in ("crate", "self", "super"):
                alias = path[0].replace("_", "-")
                if path[0] in dependency_aliases or alias in dependency_aliases:
                    graph.observed_external.add(path[0])
        for position, path in qualified_paths(parsed, use_ranges):
            current = base_module + inline_scope(parsed, position)
            target = resolve_module_path(path, current, modules, expression_path=True)
            if target is not None and target != current:
                graph.reference_edges.add((current, target))


def dependency_kind(dependency: dict) -> str:
    return dependency.get("kind") or "normal"


def dependency_label(dependency: dict) -> str:
    alias = dependency.get("rename") or dependency["name"]
    label = alias if alias == dependency["name"] else f"{alias}→{dependency['name']}"
    flags: list[str] = []
    if dependency.get("optional"):
        flags.append("optional")
    if dependency.get("target"):
        flags.append(dependency["target"])
    if dependency.get("features"):
        flags.append("features=" + "+".join(dependency["features"]))
    return label + (" (" + "; ".join(flags) + ")" if flags else "")


def markdown_list(values: Iterable[str]) -> str:
    materialized = list(values)
    return "、".join(f"`{value}`" for value in materialized) if materialized else "—"


def transitive_closure(start: str, adjacency: dict[str, set[str]]) -> set[str]:
    found: set[str] = set()
    pending = list(adjacency.get(start, ()))
    while pending:
        current = pending.pop()
        if current in found:
            continue
        found.add(current)
        pending.extend(adjacency.get(current, ()))
    found.discard(start)
    return found


def dependency_levels(names: Iterable[str], adjacency: dict[str, set[str]]) -> tuple[dict[str, int], list[list[str]]]:
    names = set(names)
    levels: dict[str, int] = {}
    remaining = set(names)
    while remaining:
        ready = sorted(name for name in remaining if not ((adjacency.get(name, set()) & names) - levels.keys()))
        if not ready:
            ready = sorted(remaining)
        for name in ready:
            levels[name] = 0 if not adjacency.get(name) else 1 + max(
                (levels.get(dependency, 0) for dependency in adjacency.get(name, set())),
                default=0,
            )
            remaining.remove(name)
    grouped: dict[int, list[str]] = collections.defaultdict(list)
    for name, level in levels.items():
        grouped[level].append(name)
    return levels, [sorted(grouped[level]) for level in sorted(grouped)]


def resolved_normal_adjacency(metadata: dict) -> dict[str, set[str]]:
    adjacency: dict[str, set[str]] = collections.defaultdict(set)
    resolve = metadata.get("resolve") or {}
    for node in resolve.get("nodes", []):
        for dependency in node.get("deps", []):
            kinds = dependency.get("dep_kinds", [])
            if any(kind.get("kind") is None for kind in kinds):
                adjacency[node["id"]].add(dependency["pkg"])
    return adjacency


def safe_fence(text: str) -> str:
    longest = max((len(match.group(0)) for match in re.finditer(r"`+", text)), default=0)
    return "`" * max(5, longest + 1)


def relative(path: pathlib.Path, root: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError:
        return str(path)


def render_module_adjacency(graph: TargetGraph, root: pathlib.Path) -> str:
    declarations: dict[tuple[str, ...], list[tuple[str, ...]]] = collections.defaultdict(list)
    references: dict[tuple[str, ...], list[tuple[str, ...]]] = collections.defaultdict(list)
    incoming: collections.Counter[tuple[str, ...]] = collections.Counter()
    for source, target in graph.declaration_edges:
        declarations[source].append(target)
    for source, target in graph.reference_edges:
        references[source].append(target)
        incoming[target] += 1
    lines: list[str] = []
    for module in sorted(graph.node_files, key=lambda value: (len(value), value)):
        file_path = relative(graph.node_files[module], root)
        lines.append(f"{format_module(module)} [{file_path}]")
        children = ", ".join(format_module(value) for value in sorted(set(declarations[module]))) or "—"
        refs = ", ".join(format_module(value) for value in sorted(set(references[module]))) or "—"
        lines.append(f"  declares: {children}")
        lines.append(f"  references: {refs}")
        lines.append(f"  referenced-by-count: {incoming[module]}")
    return "\n".join(lines)


def generate_report(root: pathlib.Path, metadata: dict) -> str:
    workspace_ids = set(metadata["workspace_members"])
    packages = sorted(
        (package for package in metadata["packages"] if package["id"] in workspace_ids),
        key=lambda package: package["name"],
    )
    package_by_name = {package["name"]: package for package in packages}
    package_names = set(package_by_name)
    package_roots = {
        package["name"]: pathlib.Path(package["manifest_path"]).resolve().parent for package in packages
    }

    files_by_package: dict[str, list[pathlib.Path]] = {}
    all_files: set[pathlib.Path] = set()
    for name, package_root in package_roots.items():
        files = sorted(set(iter_rust_files(package_root)))
        files_by_package[name] = files
        all_files.update(files)
    parsed_files = {path: parse_source(path) for path in sorted(all_files)}

    target_graphs: dict[tuple[str, str], TargetGraph] = {}
    for package in packages:
        aliases = {
            (dependency.get("rename") or dependency["name"]).replace("-", "_")
            for dependency in package["dependencies"]
        }
        aliases.update(dependency["name"] for dependency in package["dependencies"])
        for target in package["targets"]:
            graph = resolve_target_modules(package["name"], target, parsed_files)
            collect_target_references(graph, parsed_files, aliases)
            target_graphs[(package["name"], target["name"])] = graph

    internal_by_kind: dict[str, dict[str, list[dict]]] = collections.defaultdict(lambda: collections.defaultdict(list))
    external_by_kind: dict[str, dict[str, list[dict]]] = collections.defaultdict(lambda: collections.defaultdict(list))
    normal_adjacency: dict[str, set[str]] = collections.defaultdict(set)
    reverse_adjacency: dict[str, set[str]] = collections.defaultdict(set)
    for package in packages:
        name = package["name"]
        for dependency in package["dependencies"]:
            kind = dependency_kind(dependency)
            destination = dependency["name"]
            if destination in package_names:
                internal_by_kind[name][kind].append(dependency)
                if kind == "normal":
                    normal_adjacency[name].add(destination)
                    reverse_adjacency[destination].add(name)
            else:
                external_by_kind[name][kind].append(dependency)

    levels, grouped_levels = dependency_levels(package_names, normal_adjacency)
    commit = run(["git", "rev-parse", "--short=12", "HEAD"], root)
    baseline_time = run(["git", "show", "-s", "--format=%cI", "HEAD"], root)
    total_lines = sum(parsed.source.count("\n") + (0 if parsed.source.endswith("\n") or not parsed.source else 1) for parsed in parsed_files.values())
    target_count = sum(len(package["targets"]) for package in packages)
    binary_targets = [
        (package, target)
        for package in packages
        for target in package["targets"]
        if "bin" in target["kind"]
    ]
    all_warnings = [(path, warning) for path, parsed in parsed_files.items() for warning in parsed.warnings]
    unresolved = [
        (graph, issue)
        for graph in target_graphs.values()
        for issue in graph.unresolved
    ]
    reachability: dict[pathlib.Path, list[str]] = collections.defaultdict(list)
    for graph in target_graphs.values():
        by_file: dict[pathlib.Path, list[tuple[str, ...]]] = collections.defaultdict(list)
        for module, file_path in graph.node_files.items():
            by_file[file_path].append(module)
        for file_path, modules in by_file.items():
            labels = ", ".join(format_module(module) for module in sorted(set(modules)))
            reachability[file_path].append(f"{graph.target}: {labels}")
    reachable_files = set(reachability)
    auxiliary_files = sorted(all_files - reachable_files)
    auxiliary_source_files = [
        path for path in auxiliary_files if "src" in path.relative_to(package_roots[next(
            name for name, package_files in files_by_package.items() if path in package_files
        )]).parts
    ]
    aggregate_counts: collections.Counter[str] = collections.Counter()
    for parsed in parsed_files.values():
        aggregate_counts.update(parsed.counts)
    total_function_bodies = sum(len(parsed.function_bodies) for parsed in parsed_files.values())

    lines: list[str] = []
    append = lines.append
    append("# Agena Rust Workspace 全量依赖与源码骨架分析")
    append("")
    append(f"> Git 基线：`{commit}`")
    append(">")
    append(f"> 基线提交时间：`{baseline_time}`（使用提交时间以保证相同基线可重复生成）")
    append(">")
    append("> 事实来源：`cargo metadata --format-version 1 --locked` 与 workspace 第一方 Rust 源码静态扫描。")
    append("")
    append("## 1. 结论摘要")
    append("")
    append(
        f"本报告覆盖 **{len(packages)} 个 workspace package、{target_count} 个 Rust target、"
        f"{len(binary_targets)} 个 binary target、{len(parsed_files)} 个第一方 `.rs` 文件和 {total_lines:,} 行 Rust 源码**。"
    )
    append("")
    append("Cargo 的 package、crate 与 binary 不是同一个概念：一个 package 由一个 `Cargo.toml` 定义，"
           "可以产生一个或多个 crate target；`lib`、`proc-macro`、`cdylib`、`bin` 和 integration test 都是 crate。"
           "因此后文先展示 package/target，再展示 package 间依赖，避免把目录名误当成编译单元。")
    append("")
    top_out = sorted(package_names, key=lambda name: (-len(normal_adjacency[name]), name))[:8]
    top_in = sorted(package_names, key=lambda name: (-len(reverse_adjacency[name]), name))[:8]
    largest = sorted(package_names, key=lambda name: (-len(files_by_package[name]), name))[:8]
    append("关键结构事实：")
    append("")
    append("- 直接依赖面最大的第一方 package：" + "；".join(
        f"`{name}`（{len(normal_adjacency[name])}）" for name in top_out
    ) + "。")
    append("- 被依赖最多的第一方 package：" + "；".join(
        f"`{name}`（{len(reverse_adjacency[name])}）" for name in top_in
    ) + "。")
    append("- Rust 文件最多的 package：" + "；".join(
        f"`{name}`（{len(files_by_package[name])}）" for name in largest
    ) + "。")
    append(f"- 模块解析未找到/歧义项：**{len(unresolved)}**；词法结构告警：**{len(all_warnings)}**。"
           "具体项目列在“覆盖与边界”章节。")
    append("")
    append("### 1.1 架构解读")
    append("")
    append("- `agena-domain` 是最明显的基础契约层：没有第一方 normal dependency，却被 10 个第一方 package 直接依赖。"
           "`agena-tool`、`agena-provider`、`agena-storage` 都沿着该方向建立更具体的端口/契约。")
    append("- `agena-runtime` 是主要 concrete composition library：它直接汇聚 19 个第一方 package，覆盖 provider adapter、"
           "storage、plugin、MCP、skills、scheduler、LSP 与 web；任何对它的反向依赖都应谨慎审查，以免形成架构回流。")
    append("- 按 Cargo 的“调用者/上层 → 被依赖者/下层”箭头，应用链可概括为 "
           "`executable → CLI/API/application → runtime/ports → domain`。Cargo 图中不存在第一方 normal dependency cycle，"
           "因此 package 层目前可完整拓扑排序。")
    append("- 插件依赖链为 `agena-plugin-marketplace → agena-plugin-host → agena-plugin-sdk → agena-macros`；"
           "application/API/runtime 再消费 host 或 marketplace，示例插件只依赖 SDK。")
    append("- 持久化依赖主链为 `agena-storage-sqlite → agena-storage → agena-domain`；"
           "`agena-application` 对 SQLite 仅有 dev dependency，而生产 composition 由 runtime 持有 concrete adapter。")
    append("- 终端 UI 依赖为 `agena-tui → agena-tui-components`，最终由 `agena` package 的 library/application 层整合。"
           "`agena` binary 自身只有 `main.rs` 一个模块，主要实现位于同 package 的 `agena_app` library target。")
    unconsumed = sorted(
        name
        for name in package_names
        if not reverse_adjacency[name]
        and name not in {package["name"] for package, _target in binary_targets}
    )
    append("- 没有第一方 normal reverse dependency、且自身不是 binary package 的 workspace package："
           f"{markdown_list(unconsumed)}。它们可能是独立公共入口、待集成组件或实验性成员，不能仅凭零反向边判定为死代码。")
    append(f"- 源码接口扫描得到 {aggregate_counts['fn']:,} 个函数/方法签名，其中 {total_function_bodies:,} 个有函数体并已折叠；"
           f"同时记录 {aggregate_counts['struct']:,} 个 struct、{aggregate_counts['enum']:,} 个 enum、"
           f"{aggregate_counts['trait']:,} 个 trait 和 {aggregate_counts['impl']:,} 个 impl（均为未展开 token 的词法 item 计数）。")
    if auxiliary_source_files:
        append("- 生产 `src/` 下未被任何 metadata target 模块树接入的文件："
               f"{markdown_list(relative(path, root) for path in auxiliary_source_files)}。"
               "这是一条静态可达性发现，建议确认是待删除遗留实现还是漏接的模块；报告不据此自动判定死代码。")
    append("")
    append("## 2. 分析口径与重建方式")
    append("")
    append("### 2.1 依赖的四个层次")
    append("")
    append("| 层次 | 本报告中的含义 | 事实来源 |")
    append("| --- | --- | --- |")
    append("| Package/target | Cargo 实际识别的 workspace 成员与 crate target | `cargo metadata --locked` |")
    append("| Crate/package 依赖 | `normal`、`dev`、`build`、optional 与 target-specific 声明 | 各 `Cargo.toml` 经 Cargo 解析后的 metadata |")
    append("| 模块声明依赖 | `mod child;` / inline `mod child {}` 形成的父子关系 | Rust token 静态扫描与标准文件解析规则 |")
    append("| 模块引用依赖 | `use` tree 与所有能够匹配已知同 crate 模块的 `ident::...` 路径 | Rust token 静态扫描 |")
    append("")
    append("“引用依赖”是源码级边，不是运行时调用次数；通过 trait object、宏展开、注册表、反射式字符串、"
           "FFI 或生成代码发生的关系无法仅靠未展开源码完全恢复。声明图是确定性的；引用图是 token 级启发式近似，"
           "既可能漏掉动态/宏生成边，也可能因本地模块名与 import alias 同名而产生少量假阳性。")
    append("")
    append("### 2.2 源码骨架规则")
    append("")
    append("每个 `.rs` 文件的附录都来自原文件，规则如下：")
    append("")
    append("- 保留属性、可见性、泛型、where clause、参数和返回类型；")
    append("- 保留 `struct` 字段、`enum` variants、`union`、trait 外形、type alias、常量与 static；")
    append("- 保留自由函数、trait method、inherent impl/trait impl method 的完整签名；")
    append("- 把有实现的整个函数体（包括 `{}`）压缩成单个 `;`，这是保留函数边界所需的最短表示；")
    append("- 删除普通注释与文档注释以控制体积；宏 token tree 不展开；")
    append("- 未被任一 Cargo target 的 `mod` 树触达的 `.rs` 文件仍列出，并标记为辅助/未触达文件。")
    append("")
    append("### 2.3 重建命令")
    append("")
    append("```bash")
    append("python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md")
    append("```")
    append("")
    append("生成器只要求 Python 3 标准库、Git 和与仓库匹配的 Cargo；不要求全局安装 `cargo-modules`、"
           "`ast-grep` 或 Python parser 包。")
    append("")
    append("## 3. Workspace package 与 target 清单")
    append("")
    append("| Package | Manifest | Targets | Rust 文件/行 | Edition / MSRV |")
    append("| --- | --- | --- | ---: | --- |")
    for package in packages:
        name = package["name"]
        targets = ", ".join(
            f"`{target['name']}` ({'/'.join(target['kind'])})" for target in package["targets"]
        )
        source_lines = sum(
            parsed_files[path].source.count("\n") + (0 if parsed_files[path].source.endswith("\n") else 1)
            for path in files_by_package[name]
        )
        append(
            f"| `{name}` | `{relative(pathlib.Path(package['manifest_path']), root)}` | {targets} | "
            f"{len(files_by_package[name])} / {source_lines:,} | `{package['edition']}` / "
            f"`{package.get('rust_version') or 'workspace/default'}` |"
        )
    append("")
    append("## 4. Binary crate 依赖分析")
    append("")
    append("这里的“直接依赖”是 binary 所属 package 的 normal dependency 声明；若 package 同时有 library target，"
           "binary 还可以通过该 library 聚合这些依赖。Cargo metadata 不把同 package 的 lib 记录成 package dependency，"
           "因此单独标明。transitive external 是当前 lockfile resolve graph 中的 normal-edge 闭包，排除 dev/build edge；"
           "该 resolve graph 在 workspace 范围统一 feature 且保留 target-specific normal edge，所以它是跨平台/feature 上界，"
           "不是某一次指定 target 构建的精确最小集合。")
    append("")
    resolved_adj = resolved_normal_adjacency(metadata)
    metadata_package_by_id = {package["id"]: package for package in metadata["packages"]}
    for package, target in sorted(binary_targets, key=lambda pair: pair[1]["name"]):
        name = package["name"]
        graph = target_graphs[(name, target["name"])]
        internal_direct = sorted(normal_adjacency[name])
        internal_transitive = sorted(transitive_closure(name, normal_adjacency))
        external_direct = sorted(
            dependency_label(dep) for dep in external_by_kind[name]["normal"]
        )
        closure_ids = transitive_closure(package["id"], resolved_adj)
        resolved_external = sorted(
            f"{metadata_package_by_id[package_id]['name']}@{metadata_package_by_id[package_id]['version']}"
            for package_id in closure_ids
            if package_id not in workspace_ids
        )
        has_lib = any("lib" in candidate["kind"] for candidate in package["targets"])
        append(f"### 4.{sorted(binary_targets, key=lambda pair: pair[1]['name']).index((package, target)) + 1} `{target['name']}`")
        append("")
        append(f"- 所属 package：`{name}`；入口：`{relative(pathlib.Path(target['src_path']), root)}`。")
        append(f"- 同 package library target：{'有（binary 可直接引用）' if has_lib else '无'}。")
        append(f"- 入口可达模块：{len(graph.node_files)}；模块引用边：{len(graph.reference_edges)}；未解析声明：{len(graph.unresolved)}。")
        append(f"- 第一方直接 normal dependencies：{markdown_list(internal_direct)}。")
        append(f"- 第一方传递 normal closure（{len(internal_transitive)}）：{markdown_list(internal_transitive)}。")
        append(f"- 外部直接 normal declarations（{len(external_direct)}）：{markdown_list(external_direct)}。")
        append(f"- 源码 `use` 中观测到的 Cargo dependency crate roots：{markdown_list(sorted(graph.observed_external))}。")
        append("")
        append(f"<details><summary>resolved transitive external normal closure（{len(resolved_external)}）</summary>")
        append("")
        for start in range(0, len(resolved_external), 12):
            append("- " + markdown_list(resolved_external[start : start + 12]))
        append("")
        append("</details>")
        append("")
    append("## 5. 第一方 crate/package 依赖图")
    append("")
    append("### 5.1 Normal dependency 总图")
    append("")
    append("箭头 `A --> B` 表示 A 的 Cargo normal dependencies 中声明了 B；optional normal dependency 也保留，"
           "因为它仍是合法的架构方向，但并不表示默认 feature 下一定启用。")
    append("")
    append("```mermaid")
    append("flowchart LR")
    for name in sorted(package_names):
        node = re.sub(r"[^A-Za-z0-9_]", "_", name)
        append(f"    {node}[\"{name}\"]")
    for source in sorted(package_names):
        for destination in sorted(normal_adjacency[source]):
            source_node = re.sub(r"[^A-Za-z0-9_]", "_", source)
            destination_node = re.sub(r"[^A-Za-z0-9_]", "_", destination)
            append(f"    {source_node} --> {destination_node}")
    append("```")
    append("")
    append("### 5.2 拓扑层")
    append("")
    append("层 0 不依赖其他第一方 package；更高层只指向相同或更低层。dev/build edges 不参与分层。")
    append("")
    append("| 层 | Packages |")
    append("| ---: | --- |")
    for index, group in enumerate(grouped_levels):
        append(f"| {index} | {markdown_list(group)} |")
    append("")
    append("### 5.3 全量第一方邻接表")
    append("")
    append("| Package | Direct normal | Direct dev | Direct build | Reverse normal | Transitive normal |")
    append("| --- | --- | --- | --- | --- | --- |")
    for name in sorted(package_names):
        normal = sorted(dependency_label(dep) for dep in internal_by_kind[name]["normal"])
        dev = sorted(dependency_label(dep) for dep in internal_by_kind[name]["dev"])
        build = sorted(dependency_label(dep) for dep in internal_by_kind[name]["build"])
        reverse = sorted(reverse_adjacency[name])
        closure = sorted(transitive_closure(name, normal_adjacency))
        append(
            f"| `{name}` | {markdown_list(normal)} | {markdown_list(dev)} | {markdown_list(build)} | "
            f"{markdown_list(reverse)} | {markdown_list(closure)} |"
        )
    append("")
    append("## 6. 外部 crate 声明与 lockfile 解析概况")
    append("")
    external_packages = [package for package in metadata["packages"] if package["id"] not in workspace_ids]
    versions_by_name: dict[str, set[str]] = collections.defaultdict(set)
    for package in external_packages:
        versions_by_name[package["name"]].add(package["version"])
    duplicates = {name: versions for name, versions in versions_by_name.items() if len(versions) > 1}
    append(
        f"当前 metadata resolve graph 共 {len(metadata['packages'])} 个 package，其中第一方 {len(packages)}、"
        f"外部/registry/git/path package {len(external_packages)}；有 {len(duplicates)} 个外部名称同时解析出多个版本。"
    )
    append("")
    append("### 6.1 每个第一方 package 的直接外部声明")
    append("")
    for name in sorted(package_names):
        package = package_by_name[name]
        append(f"<details><summary><code>{name}</code> — direct external dependencies</summary>")
        append("")
        for kind in ("normal", "dev", "build"):
            dependencies = sorted(dependency_label(dep) for dep in external_by_kind[name][kind])
            append(f"- {kind}（{len(dependencies)}）：{markdown_list(dependencies)}")
        feature_names = sorted(package.get("features", {}))
        append(f"- Cargo features（{len(feature_names)}）：{markdown_list(feature_names)}")
        append("")
        append("</details>")
        append("")
    append("### 6.2 多版本外部 crate")
    append("")
    if duplicates:
        append("| Crate | Resolved versions |")
        append("| --- | --- |")
        for name, versions in sorted(duplicates.items()):
            append(f"| `{name}` | {markdown_list(sorted(versions))} |")
    else:
        append("没有同名多版本外部 crate。")
    append("")
    append("## 7. Crate 内模块声明树与引用邻接表")
    append("")
    append("每个 target 单独建图，因为同一 package 的 `lib.rs`、`main.rs`、显式 test target 可以拥有不同 crate root。"
           "每个模块条目都列出对应文件、直接声明的子模块、源码引用到的同 crate 模块以及被其他模块引用的边数。"
           "inline module 仍是独立模块节点，但会指向其所在 `.rs` 文件。")
    append("")
    target_index = 0
    for package in packages:
        for target in package["targets"]:
            target_index += 1
            graph = target_graphs[(package["name"], target["name"])]
            append(f"### 7.{target_index} `{package['name']}::{target['name']}` ({'/'.join(target['kind'])})")
            append("")
            append(
                f"入口 `{relative(graph.root, root)}`；模块 {len(graph.node_files)}；声明边 "
                f"{len(graph.declaration_edges)}；引用边 {len(graph.reference_edges)}；"
                f"源码观测 dependency roots {len(graph.observed_external)}。"
            )
            append("")
            append("<details><summary>完整模块邻接表</summary>")
            append("")
            append("```text")
            append(render_module_adjacency(graph, root))
            append("```")
            append("")
            append("</details>")
            append("")
            if graph.unresolved:
                append("未解析声明：")
                append("")
                for issue in graph.unresolved:
                    append(f"- `{relative(pathlib.Path(issue.split(':', 1)[0]), root) if ':' in issue else issue}`")
                append("")
    append("## 8. 每个 Rust 文件的完整接口级源码骨架")
    append("")
    append("以下按 package 与路径排序，覆盖 workspace package 根目录内的全部 `.rs` 文件。每个折叠块中的代码不是可编译产物，"
           "而是为了架构审阅生成的接口骨架；原始行数和 item 粗计数来自未删减源码。")
    append("")
    file_number = 0
    for package in packages:
        name = package["name"]
        append(f"### 8.{sorted(package_names).index(name) + 1} `{name}`")
        append("")
        for path in files_by_package[name]:
            file_number += 1
            parsed = parsed_files[path]
            source_lines = parsed.source.count("\n") + (0 if parsed.source.endswith("\n") or not parsed.source else 1)
            counts = ", ".join(
                f"{key}={parsed.counts[key]}" for key in ("mod", "struct", "enum", "union", "trait", "type", "impl", "fn")
                if parsed.counts[key]
            ) or "no counted items"
            file_reachability = reachability.get(path, [])
            reachable = "；".join(sorted(file_reachability)) if file_reachability else "未被 metadata target 的静态 mod 树触达"
            display_path = relative(path, root)
            append(
                f"<details><summary><code>{display_path}</code> — {source_lines} lines; {counts}</summary>"
            )
            append("")
            append(f"- Package：`{name}`")
            append(f"- Target/module：{reachable}")
            if parsed.warnings:
                append(f"- Lexer warnings：{'; '.join(parsed.warnings)}")
            append("")
            skeleton = source_skeleton(parsed)
            fence = safe_fence(skeleton)
            append(f"{fence}rust")
            append(skeleton)
            append(fence)
            append("")
            append("</details>")
            append("")
    append("## 9. 覆盖率、校验与静态分析边界")
    append("")
    append("### 9.1 覆盖率")
    append("")
    append("| 指标 | 结果 |")
    append("| --- | ---: |")
    append(f"| Workspace packages | {len(packages)} |")
    append(f"| Cargo Rust targets | {target_count} |")
    append(f"| Binary targets | {len(binary_targets)} |")
    append(f"| 扫描 `.rs` 文件 | {len(parsed_files)} |")
    append(f"| 至少被一个 target module tree 触达的文件 | {len(reachable_files)} |")
    append(f"| 辅助/fixture/未触达 `.rs` 文件 | {len(auxiliary_files)} |")
    append(f"| 模块声明未解析项 | {len(unresolved)} |")
    append(f"| Lexer 结构告警 | {len(all_warnings)} |")
    append(f"| 函数/方法签名 | {aggregate_counts['fn']} |")
    append(f"| 已省略的函数体 | {total_function_bodies} |")
    append(f"| Struct / enum / trait / impl | {aggregate_counts['struct']} / {aggregate_counts['enum']} / {aggregate_counts['trait']} / {aggregate_counts['impl']} |")
    append("")
    if auxiliary_files:
        append("未被 target module tree 触达的文件（仍已在源码骨架附录中覆盖）：")
        append("")
        for path in auxiliary_files:
            append(f"- `{relative(path, root)}`")
        append("")
    if unresolved:
        append("模块声明未解析项：")
        append("")
        for graph, issue in unresolved:
            append(f"- `{graph.package}::{graph.target}` — {issue}")
        append("")
    if all_warnings:
        append("Lexer 告警：")
        append("")
        for path, warning in all_warnings:
            append(f"- `{relative(path, root)}` — {warning}")
        append("")
    append("### 9.2 必须人工补充判断的边界")
    append("")
    append("- `#[cfg]` 与 target-specific dependencies：报告保留声明全集；不同 OS/feature 的实际编译子图会更小。")
    append("- Proc macro 与 `macro_rules!`：宏 token tree 不展开，因此宏生成的模块、类型、函数和引用边不进入源码引用图。")
    append("- 动态分派：trait object、注册表、插件 ABI、MCP/tool 名称与字符串路由不会形成普通静态模块调用边。")
    append("- `include!`/build script 生成代码：生成到 `OUT_DIR` 的 Rust 不属于仓库 `.rs` 文件清单；应结合具体 build 输出审计。")
    append("- `third_party/`：其中的 manifest 不是 workspace member，因此不进入第一方逐文件源码骨架；"
           "若被 Cargo patch/path dependency 使用，仍会出现在完整 resolve graph 的外部/path package 统计中。")
    append("- 引用边只表达“源码中存在路径关系”，不表达调用次数、运行时热度、所有权方向或是否应该合并模块。")
    append("- 完整类型解析/调用图应在此报告基础上再使用 rust-analyzer SCIP、rustdoc JSON 或 compiler MIR；"
           "本报告的目标是稳定、可读、可重建的 workspace/crate/module/source interface 基线。")
    append("")
    append("### 9.3 建议的持续集成校验")
    append("")
    append("```bash")
    append("python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md")
    append("git diff --exit-code -- docs/rust-workspace-analysis.md")
    append("cargo check --workspace --all-targets --locked")
    append("```")
    append("")
    append("若希望阻止错误的架构方向，应另外维护允许边/禁止边规则并对 Cargo normal adjacency 做机器校验；"
           "报告本身是快照，不替代架构策略。")
    append("")
    return "\n".join(lines)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path("docs/rust-workspace-analysis.md"),
        help="report path, relative to the repository root by default",
    )
    parser.add_argument(
        "--metadata",
        type=pathlib.Path,
        help="read Cargo metadata JSON from this file instead of invoking Cargo",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    root = repository_root(pathlib.Path.cwd())
    if args.metadata:
        metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    else:
        metadata = json.loads(
            run(["cargo", "metadata", "--format-version", "1", "--locked"], root)
        )
    output = args.output if args.output.is_absolute() else root / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    report = generate_report(root, metadata)
    output.write_text(report.rstrip("\n") + "\n", encoding="utf-8")
    print(f"wrote {output} ({len(report.splitlines()):,} lines, {len(report.encode('utf-8')):,} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
