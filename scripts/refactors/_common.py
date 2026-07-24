#!/usr/bin/env python3
"""Shared safety helpers for one-shot repository refactor tools."""

from __future__ import annotations

import difflib
import json
import os
import pathlib
import stat
import tempfile
from collections.abc import Iterable, Mapping
from typing import Any


class RefactorError(RuntimeError):
    """A user-actionable validation failure."""


def load_manifest(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise RefactorError(f"manifest does not exist: {path}") from error
    except json.JSONDecodeError as error:
        raise RefactorError(f"invalid JSON manifest {path}: {error}") from error
    if not isinstance(value, dict):
        raise RefactorError(f"manifest root must be an object: {path}")
    if value.get("version") != 1:
        raise RefactorError(f"manifest {path} must contain version: 1")
    return value


def require_list(value: Any, field: str) -> list[Any]:
    if not isinstance(value, list):
        raise RefactorError(f"{field} must be an array")
    return value


def require_string(value: Any, field: str, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value):
        suffix = "string" if allow_empty else "non-empty string"
        raise RefactorError(f"{field} must be a {suffix}")
    return value


def resolve_root(raw: pathlib.Path | None) -> pathlib.Path:
    return (raw or pathlib.Path.cwd()).resolve()


def resolve_within(root: pathlib.Path, raw: str, field: str) -> pathlib.Path:
    relative = pathlib.Path(require_string(raw, field))
    if relative.is_absolute():
        raise RefactorError(f"{field} must be relative to the refactor root: {raw}")
    resolved = (root / relative).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise RefactorError(f"{field} escapes the refactor root: {raw}") from error
    return resolved


def relative_display(path: pathlib.Path, root: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError:
        return str(path)


def unified_diff(
    path: pathlib.Path,
    before: str,
    after: str,
    root: pathlib.Path,
) -> str:
    label = relative_display(path, root)
    return "".join(
        difflib.unified_diff(
            before.splitlines(keepends=True),
            after.splitlines(keepends=True),
            fromfile=f"a/{label}",
            tofile=f"b/{label}",
        )
    )


def render_diffs(
    changes: Mapping[pathlib.Path, str],
    root: pathlib.Path,
) -> str:
    chunks: list[str] = []
    for path in sorted(changes):
        before = path.read_text(encoding="utf-8") if path.exists() else ""
        chunks.append(unified_diff(path, before, changes[path], root))
    return "".join(chunks)


def _write_staged(path: pathlib.Path, content: str, mode: int | None) -> pathlib.Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, raw_temp = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".refactor-tmp", dir=path.parent
    )
    temp = pathlib.Path(raw_temp)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        if mode is not None:
            os.chmod(temp, stat.S_IMODE(mode))
        return temp
    except BaseException:
        temp.unlink(missing_ok=True)
        raise


def atomic_write_many(changes: Mapping[pathlib.Path, str]) -> None:
    """Validate and replace a group of UTF-8 files with best-effort rollback."""
    if not changes:
        return

    originals: dict[pathlib.Path, tuple[bytes | None, int | None]] = {}
    staged: dict[pathlib.Path, pathlib.Path] = {}
    replaced: list[pathlib.Path] = []
    try:
        for path, content in changes.items():
            if path.exists() and not path.is_file():
                raise RefactorError(f"refusing to replace non-file path: {path}")
            original = path.read_bytes() if path.exists() else None
            mode = path.stat().st_mode if path.exists() else None
            originals[path] = (original, mode)
            staged[path] = _write_staged(path, content, mode)

        for path in sorted(staged):
            os.replace(staged[path], path)
            replaced.append(path)
    except BaseException:
        for temp in staged.values():
            temp.unlink(missing_ok=True)
        for path in reversed(replaced):
            original, mode = originals[path]
            if original is None:
                path.unlink(missing_ok=True)
                continue
            restore = _write_staged(path, original.decode("utf-8"), mode)
            os.replace(restore, path)
        raise


def find_matching_files(
    root: pathlib.Path,
    roots: Iterable[str],
    includes: Iterable[str],
    excludes: Iterable[str],
) -> list[pathlib.Path]:
    import fnmatch

    include_patterns = list(includes) or ["**/*"]
    exclude_patterns = list(excludes)
    found: set[pathlib.Path] = set()

    def matches(path: str, patterns: list[str]) -> bool:
        for pattern in patterns:
            if fnmatch.fnmatch(path, pattern):
                return True
            if pattern.startswith("**/") and fnmatch.fnmatch(path, pattern[3:]):
                return True
        return False

    for raw_root in roots:
        base = resolve_within(root, raw_root, "roots[]")
        if not base.exists():
            raise RefactorError(f"search root does not exist: {relative_display(base, root)}")
        candidates = [base] if base.is_file() else base.rglob("*")
        for candidate in candidates:
            if not candidate.is_file():
                continue
            relative = relative_display(candidate, root)
            if matches(relative, include_patterns) and not matches(relative, exclude_patterns):
                found.add(candidate)
    return sorted(found)
