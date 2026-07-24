# Refactor Helpers

These standard-library-only tools support the repository's continuous source
train. They are deliberately conservative: all mutating commands are dry-run
by default, paths must stay below an explicit root, and writes happen only
after every manifest assertion succeeds.

The tools do not decide architecture. A reviewed plan must still decide which
crate or module owns each Rust item.

## `split-rust-file.py`

This tool reuses the lexer and delimiter parser in
`scripts/rust-architecture-report.py`. It inventories movable top-level Rust
items and can extract selected items into new files without retyping their
bodies.

List stable selectors first:

```bash
python3 scripts/refactors/split-rust-file.py \
  --root . \
  list \
  --source crates/agena-tui-transcript/src/renderer/transcript_ast.rs
```

Machine-readable inventory:

```bash
python3 scripts/refactors/split-rust-file.py \
  --root . \
  list \
  --source crates/agena-tui-transcript/src/renderer/transcript_ast.rs \
  --json
```

Example split manifest:

```json
{
  "version": 1,
  "source": "crates/example/src/widget.rs",
  "destinations": [
    {
      "path": "crates/example/src/widget/types.rs",
      "header": "use super::*;",
      "items": [
        "struct:Widget",
        "enum:WidgetState",
        "impl@1"
      ]
    },
    {
      "path": "crates/example/src/widget/tests.rs",
      "header": "use super::*;",
      "items": [
        "mod:tests"
      ]
    }
  ]
}
```

Validate and preview without writing:

```bash
python3 scripts/refactors/split-rust-file.py \
  --root . \
  split \
  --manifest /tmp/widget-split.json \
  --show-diff
```

Apply the already validated operation:

```bash
python3 scripts/refactors/split-rust-file.py \
  --root . \
  split \
  --manifest /tmp/widget-split.json \
  --apply
```

Safety properties:

- selectors must match the inventory exactly;
- one selector cannot be sent to multiple destinations;
- item byte ranges must not overlap;
- destination files must not already exist;
- source and every destination are staged before replacement;
- outer attributes, visibility modifiers, and immediately attached comments
  move with the selected item;
- the file's leading shebang, inner attributes, and inner docs stay at the
  source root;
- an error before or during replacement triggers best-effort rollback.

The splitter does not guess imports or public APIs. Use destination `header`
for reviewed imports, then update the source module declarations explicitly.

## `assert-replace.py`

This tool performs literal replacements only in an explicit file list. Every
replacement group declares the exact number of old occurrences it expects.
All groups are simulated in memory before any file is written.

Example manifest:

```json
{
  "version": 1,
  "replacements": [
    {
      "files": [
        "Cargo.toml",
        "apps/agena/Cargo.toml",
        "apps/agena/src/server/state.rs"
      ],
      "old": "agena-studio-git",
      "new": "agena-git-http",
      "expected": 3,
      "expected_new_before": 0
    },
    {
      "files": [
        "apps/agena/src/server/state.rs"
      ],
      "old": "agena_studio_git",
      "new": "agena_git_http",
      "expected": 1,
      "expected_new_before": 0
    }
  ]
}
```

Dry-run and diff:

```bash
python3 scripts/refactors/assert-replace.py \
  --root . \
  --manifest /tmp/rename-git.json \
  --show-diff
```

Apply:

```bash
python3 scripts/refactors/assert-replace.py \
  --root . \
  --manifest /tmp/rename-git.json \
  --apply
```

Replacement groups run in manifest order. Counts for a later group are
evaluated against the in-memory result of earlier groups. A mismatch writes
nothing. A file may appear only once inside one replacement group's `files`
array, preventing duplicate paths from inflating the asserted count.

## `check-refactor-invariants.py`

This tool turns source-train static gates into a repeatable manifest. It can
assert path removal/creation, text occurrence counts, and physical line limits.

Example manifest:

```json
{
  "version": 1,
  "must_exist": [
    "apps/agena/src/main.rs",
    "docs/agena-unified-binary-and-continuous-decomposition-plan.md"
  ],
  "must_not_exist": [
    "apps/agena-studio-server"
  ],
  "text_rules": [
    {
      "name": "active Studio identifiers are gone",
      "roots": ["apps", "crates", "packages", "scripts", ".github"],
      "include": ["**/*.rs", "**/*.toml", "**/*.ts", "**/*.vue", "**/*.sh", "**/*.yml"],
      "exclude": [
        "apps/agena/src/server/persistence/legacy_studio.rs",
        "packages/agena-web-ui/src/lib/persistence/legacyStudio.ts"
      ],
      "pattern": "agena-studio|agena_studio|AGENA_STUDIO|studio-server",
      "regex": true,
      "expected": 0
    }
  ],
  "line_rules": [
    {
      "name": "production Rust files stay below the hard ceiling",
      "roots": ["apps", "crates"],
      "include": ["**/src/**/*.rs"],
      "exclude": ["**/tests/**"],
      "max_lines": 2000
    }
  ]
}
```

Run it with:

```bash
python3 scripts/refactors/check-refactor-invariants.py \
  --root . \
  --manifest /tmp/unified-binary-gates.json
```

Text rules accept an exact `expected` count, a `min`, a `max`, or a
combination of `min` and `max`. Literal matching is the default; set `regex` to
`true` only when needed.

## Tests

The tests create isolated temporary roots and never modify repository source:

```bash
python3 -m unittest discover -s scripts/refactors/tests -v
```

They cover dry-run behavior, Rust attribute/comment movement, unknown selector
failure, existing-destination protection, exact replacement counts, duplicate
input rejection, zero-write failures, invariant success, and invariant
violation reporting.
