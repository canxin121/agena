# Structural search, rewrites and language servers

Use `fs.grep` for text, `code.search_ast` for source structure, and `lsp.definition` / `lsp.references` for symbol relationships. Agena embeds the ripgrep and ast-grep libraries; those tools do not require the corresponding CLIs to be installed.

## Patterns and rules

`code.search_ast` accepts exactly one `pattern` or `rule`. The simple pattern interface remains available, including broad metavariable patterns. Structured rules use the [official ast-grep rule shape](https://ast-grep.github.io/reference/rule.html), including `kind`, `pattern`, `all`, `any`, `not`, and relationships such as `inside` / `has`.

For example, find logging calls inside a function in one JavaScript file:

```json
{
  "path": "src/example.js",
  "rule": {
    "all": [
      {"pattern": "console.log($A)"},
      {"inside": {"kind": "function_declaration", "stopBy": "end"}}
    ]
  },
  "limit": 20
}
```

Language is inferred for files. A directory search requires `language`. The rule object is bounded to 16 KiB, 16 nesting levels and 512 values; standalone patterns are bounded to 16 KiB. This interface accepts a rule object, not a complete ast-grep YAML rule file. It does not load external utility rules or execute commands.

Search results explicitly distinguish an incomplete scan (`truncated` / `truncation_reason`) from a shortened match preview (`text_truncated`). Exactly reaching `limit` does not prove truncation: an additional match does. Source reads are bounded to 8 MiB per file, 256 MiB total, 10,000 source files and 100,000 discovered entries, with a 20-second budget checked between work units. Blocking filesystem calls and a single parser/matcher operation are not forcibly preempted.

`code.syntax_tree` exposes named nodes at `max_depth` 1–6 (default 2). Its preview contains at most 512 nodes and 50 children per node. `children_truncated` marks each omitted child set, and result-level `truncated` reports any omitted descendants.

## Review and apply one structural rewrite

`code.rewrite_ast` uses the official ast-grep replacement templates and defaults to preview. A preview does not write the file:

```json
{
  "path": "src/example.js",
  "pattern": "console.log($A)",
  "replacement": "logger.info($A)"
}
```

The result includes `replacements`, a bounded unified `diff`, `diff_truncated`, `before_sha256`, and `after_sha256`. Review the diff, then repeat the request with `apply: true` and `expected_sha256` set to the returned `before_sha256`. `rule` works here too. An empty replacement deletes a match.

The tool requires one UTF-8 file and rejects an incomplete plan, overlapping matches, undefined replacement variables, parse errors in the original/result, more than 100 matches, or a result above 8 MiB. It applies all selected matches or fails before publication. A rule can produce no text change; `changed` makes that explicit.

Application uses the same canonical path, file-mutation lock, staged replacement and publication-time content comparison as the filesystem tools. A stale preview fails. An external writer that does not participate in the locking protocol can still race between the last comparison and rename; this is not a filesystem transaction against arbitrary writers. Preview calls carry the same filesystem-mutation tool classification as apply calls, while their returned effects correctly report no write.

## LSP availability and project routing

`lsp.servers` reports configured commands, file extensions, root markers, executable discovery and running project roots. Executable discovery inspects files without running the command. It uses each server's configured PATH relative to the workspace root; relative commands may resolve differently under other project roots. An unknown lookup is `command_available: null`, including Windows per-server PATHEXT overrides. Finding an executable does not guarantee successful initialization.

Extension-specific servers take precedence over catch-all servers. Ties use lexical server name, so selection is reproducible. Clients are cached per server and canonical project root. Concurrent requests to the same pair share one startup; changing a registered specification shuts down its old clients before subsequent initialization. Shutdown closes every project instance, including when callers still retain client handles.

Input positions are zero-based lines and UTF-16 code-unit offsets, as required by the default LSP position encoding. Returned display locations are one-based; file URIs are decoded to local paths, including spaces and Unicode. Diagnostics retain the existing document-version/freshness contract: empty pending or stale results do not certify that a file is clean.

Validation uses embedded ast-grep tests, the real plugin dispatch path, and a local Python stdio LSP fixture. No Agena service or connector, paid provider, or external language server is required for these tests. Live language-server semantic quality still depends on the configured server and project.
