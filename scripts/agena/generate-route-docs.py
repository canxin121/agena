#!/usr/bin/env python3
"""Regenerate crates/agena-api-server/src/route_catalog.rs.

The route catalog is a doc-only module: its page in `cargo doc` lists every
mounted route grouped by domain. Keeping it generated means the published list
cannot drift from the router.

Usage:
    python3 scripts/agena/generate-route-docs.py          # rewrite in place
    python3 scripts/agena/generate-route-docs.py --check   # fail if stale
"""

from __future__ import annotations

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates" / "agena-api-server" / "src"
OUTPUT = CRATE / "route_catalog.rs"

# Files that may contain `.route(...)` calls, in reading order.
#
# During the router split the chain lives in `routes/*.rs`; the streaming
# routes stay in `lib.rs`. Once every domain module exists, `lib.rs` no longer
# holds the REST chain, so the two sources never double-count.
SOURCES = [*(sorted((CRATE / "routes").glob("*.rs"))), CRATE / "lib.rs"]

ROUTE_START = ".route("


def route_chunks(text: str) -> list[str]:
    """Yield every balanced `.route(...)` call, including nested layers."""
    chunks: list[str] = []
    index = 0
    while True:
        start = text.find(ROUTE_START, index)
        if start < 0:
            return chunks
        pos = start + len(ROUTE_START)
        depth = 1
        while pos < len(text) and depth:
            char = text[pos]
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            pos += 1
        chunks.append(text[start:pos])
        index = pos


def domain_of(path: str) -> str:
    if path in ("/healthz", "/readyz", "/metrics"):
        return "ops"
    if path.startswith("/plugin-rpc"):
        return "plugin-rpc"
    segments = path.strip("/").split("/")
    if len(segments) >= 2 and segments[0] == "api" and segments[1] == "v1":
        return segments[2] if len(segments) > 2 else "(root)"
    return "ws/sse"


def collect() -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for source in SOURCES:
        if not source.exists():
            continue
        for chunk in route_chunks(source.read_text()):
            match = re.search(r'\.route\(\s*"([^"]+)"', chunk)
            if not match:
                continue
            path = match.group(1)
            handlers = re.findall(r"\b(get|post|put|delete|patch)\(\s*(?:rest::)?([a-z_0-9]+)", chunk)
            methods = ",".join(method.upper() for method, _ in handlers) or "?"
            names = ", ".join(f"`{name}()`" for _, name in handlers) or "—"
            rows.append((path, methods, names))
    return rows


def render(rows: list[tuple[str, str, str]]) -> str:
    grouped: dict[str, list[tuple[str, str, str]]] = collections.OrderedDict()
    for row in rows:
        grouped.setdefault(domain_of(row[0]), []).append(row)

    body: list[str] = []
    for domain in sorted(grouped):
        body.append(f"//! ### `{domain}`")
        body.append("//!")
        body.append("//! | Method | Path | Handlers |")
        body.append("//! |---|---|---|")
        for path, methods, names in grouped[domain]:
            body.append(f"//! | {methods} | `{path}` | {names} |")
        body.append("//!")
    return "\n".join(body)


HEADER = """//! # HTTP 路由目录
//!
//! 本页列出当前挂载的每一条路由，按领域分组并标注 handler。运行
//! `cargo doc -p agena-api-server --no-deps --open` 浏览，或搜索 `route`。
//!
//! **生成文件，请勿手工编辑。** 修改路由后重新运行：
//!
//! ```sh
//! python3 scripts/agena/generate-route-docs.py
//! ```
//!
//! 路由的**目标形态**（含尚未落地的重命名）见 `docs/design/api-v1-routes.md`；
//! 本页描述的是**当前实际挂载**的路由，迁移完成前两者会存在差异。
//!
//! ## 约定
//!
//! - REST handler 位于 [`crate::rest`]：解析请求、调用 application 服务、
//!   返回 JSON 资源。
//! - `/api/v1/ws`、`/api/v1/changes/stream`、`/api/v1/notifications/stream`
//!   由 [`crate::ws`] 与 [`crate::sse`] 提供，出现在下表末尾。
//! - `.layer(DefaultBodyLimit::max(...))` 逐路由挂载：只有明确需要更大请求体
//!   的路由才放宽 axum 的 2 MiB 默认值。
//!
//! ## 当前路由
//!
"""


def main() -> int:
    rows = collect()
    text = HEADER + render(rows) + "\n"
    if "--check" in sys.argv:
        current = OUTPUT.read_text() if OUTPUT.exists() else ""
        if current != text:
            print("route_catalog.rs is stale; run scripts/agena/generate-route-docs.py")
            return 1
        print(f"route catalog up to date ({len(rows)} routes)")
        return 0
    OUTPUT.write_text(text)
    print(f"wrote {OUTPUT.relative_to(ROOT)} ({len(rows)} routes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
