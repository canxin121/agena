# Rust Code Analysis Tooling

This document defines the tools and repeatable workflow used to analyze the
Agena Rust workspace. The goal is to make architecture, duplicate-code, small
function, and low-reference investigations reproducible instead of relying on
plain-text searches or subjective review.

The analysis is static. A "reference count" means the number of semantic
references found in the current workspace; it does not mean the number of
runtime invocations.

## Tool set

### `cargo metadata` and `cargo tree`

Use Cargo's own metadata as the source of truth for workspace membership,
targets, features, and dependency direction.

```bash
cargo metadata --no-deps --format-version 1
cargo tree --workspace --edges normal
cargo tree --workspace --duplicates
```

These commands answer questions such as:

- which crates and binaries belong to the workspace;
- whether a proposed shared helper would introduce an invalid dependency
  direction;
- whether the workspace contains duplicate dependency versions.

Do not infer crate boundaries only from directory names.

### `cargo-modules`

Use `cargo-modules` to inspect the Rust module hierarchy and public API shape.
It is useful for architecture orientation before function-level analysis.

```bash
cargo install cargo-modules --locked
cargo modules structure --package agena
cargo modules structure --package agena-cli
cargo modules dependencies --package agena
```

Typical uses include:

- viewing the module tree without reading every source file;
- locating large subsystems and repeated parallel module families;
- checking whether an item is public, crate-visible, or private;
- identifying a suitable owner module for shared code.

Module output does not describe call frequency or dynamic dispatch. It should
be combined with the semantic index described below.

### `rust-analyzer`

`rust-analyzer` provides syntax-aware and type-aware analysis for the whole
Cargo workspace. We use it both as a compiler-adjacent analyzer and as the
producer of a SCIP semantic index.

```bash
rustup component add rust-analyzer
rust-analyzer analysis-stats . --parallel
```

For repeatable symbol and reference analysis, generate a SCIP index outside
the repository:

```bash
analysis_dir="$(mktemp -d)"
rust-analyzer scip . \
  --output "$analysis_dir/agena.scip" \
  --exclude-vendored-libraries \
  --num-threads 8
```

Keeping the index outside the repository avoids accidentally committing a
large generated file. The index contains definitions, references, source
ranges, symbol kinds, signatures, and enclosing-symbol information.

### SCIP CLI

Use the [SCIP CLI](https://github.com/sourcegraph/scip) to inspect the semantic
index produced by `rust-analyzer`. Install a released `scip` binary and place
it on `PATH`.

```bash
scip stats --from "$analysis_dir/agena.scip"
scip print --json "$analysis_dir/agena.scip"
```

The JSON output can be processed with `jq`, `sort`, and `awk` to calculate:

- workspace-wide symbol definition counts;
- zero-reference and one-reference functions;
- definition locations for review;
- reference distributions by crate or module;
- likely dead private helpers.

A SCIP occurrence whose `symbol_roles` contains the definition bit is a
definition; other occurrences of the same symbol are references. Counts must
be grouped by the complete SCIP symbol, not only by the function name, because
different traits and types can define methods with the same display name.

Semantic reference counts have important limitations:

- a trait method call can be attributed to the trait symbol instead of a
  concrete implementation;
- Serde callbacks named in attributes may appear to have no ordinary call
  sites;
- proc macros, generated dispatchers, plugin registration, and dynamic
  dispatch can hide the effective caller;
- a public function with no workspace references may still be an external API;
- static reference count says nothing about runtime hotness.

For these reasons, reference counts produce candidates, not automatic deletion
decisions.

### `ast-grep`

Use `ast-grep` for syntax-tree queries over Rust source. It is more reliable
than regular expressions for locating complete functions, methods, and bodies.

```bash
cargo install ast-grep --locked
```

List all Rust functions under application and crate source trees:

```bash
ast-grep run \
  --kind function_item \
  --lang rust \
  --json=stream \
  crates apps \
  --globs '**/src/**/*.rs'
```

Extract only the block directly owned by each function:

```bash
ast-grep run \
  --kind 'function_item > block' \
  --lang rust \
  --json=stream \
  crates apps \
  --globs '**/src/**/*.rs'
```

The structured output includes the source file, byte range, line range,
function text, and body text. We use it to calculate:

- function span in physical lines;
- small-function candidate lists;
- exact duplicate function bodies after removing insignificant whitespace;
- the location and signature of every duplicate instance.

Our default small-function thresholds are:

- **very small:** at most 3 physical lines;
- **small:** at most 5 physical lines;
- **review range:** at most 7 physical lines when combined with zero or one
  semantic reference.

These thresholds are filters only. Trait adapters, accessors, constructors,
conversion methods, and well-named predicates can remain valuable even when
their bodies contain one expression.

For duplicate detection, start with exact normalized bodies. Do not begin with
aggressive identifier renaming or fuzzy matching: those methods produce many
false positives in provider adapters, DTO conversion code, and trait
implementations. Similar-but-not-identical clones can be reviewed after exact
duplicates have been resolved.

### `rg`

Use `ripgrep` for fast source discovery and for manually verifying candidates
reported by semantic or AST analysis.

```bash
rg --files -g '*.rs' crates apps
rg -n '\bfunction_name\b' crates apps
rg -n -U '#\[allow\(dead_code\)\]\s*(?:pub[^\n]*\s+)?(?:async\s+)?fn\s+' \
  --glob '*.rs' crates apps
```

`rg` is not the primary call-graph tool. A textual match can be a definition,
import, comment, macro input, or unrelated method with the same name. Use it
to inspect and confirm a semantic result, not to produce the final reference
count.

### Cargo validation

Every analysis-driven cleanup must finish with Cargo validation appropriate to
the changed scope.

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
```

For a small change, start with the affected package and targeted tests, then
expand to the workspace before merging. `cargo check` cannot prove that an
`#[allow(dead_code)]` function is needed, and it cannot detect semantic drift
between two compiling duplicate implementations.

## Repeatable analysis workflow

Use the following sequence when investigating the codebase.

1. **Record repository state.** Run `git status --short --branch` and preserve
   unrelated local changes.
2. **Map the workspace.** Use `cargo metadata`, `cargo tree`, and
   `cargo-modules` to understand crate and module ownership.
3. **Generate a semantic index.** Produce the `rust-analyzer` SCIP file in a
   temporary directory and review `scip stats` for coverage.
4. **Collect AST facts.** Use `ast-grep` to extract function spans, signatures,
   and bodies from `crates/**/src` and `apps/**/src`.
5. **Join by source location.** Match the SCIP definition location to the
   corresponding AST function. This combines semantic reference counts with
   precise function size and body information.
6. **Apply exclusions.** Separate public APIs, tests, trait declarations,
   trait implementations, Serde callbacks, proc-macro entry points, and plugin
   dispatch methods from ordinary private helpers.
7. **Rank candidates.** Review in this order:
   - private functions explicitly marked `#[allow(dead_code)]` with no
     workspace references;
   - exact duplicate bodies in the same crate;
   - exact duplicate bodies across crates where dependency direction permits a
     shared owner;
   - small private functions with zero or one reference;
   - similar but non-identical implementations that encode the same protocol
     or business rule.
8. **Verify manually.** Inspect definitions, callers, attributes, macro usage,
   public re-exports, and relevant tests before deleting or consolidating code.
9. **Make one coherent cleanup at a time.** Avoid mixing unrelated duplicate
   families in a single commit.
10. **Validate and report.** Run formatting, checks, Clippy, and tests in
    proportion to risk, and record what was consolidated or intentionally left
    separate.

## Candidate classification

Each reported function should be assigned one of these outcomes:

| Classification | Meaning | Typical action |
| --- | --- | --- |
| Dead | No real caller and no API or generated-entry role | Delete it and remove `allow(dead_code)` |
| Duplicate | Same behavior is maintained in more than one place | Move behavior to a single owner |
| Thin semantic wrapper | Small body, but its name expresses a useful domain boundary | Keep it or delegate to one shared implementation |
| Inline candidate | Private, one caller, and the name adds little information | Inline into the caller |
| Framework entry | Called through a trait, macro, attribute, registry, or dynamic dispatch | Keep it; document the false-positive reason if necessary |
| Public surface | Externally callable even with no workspace references | Keep unless an API-breaking removal is intentional |
| Intentionally separate | Similar implementation must evolve independently because of protocol or dependency boundaries | Keep separate and add a short rationale |

## Repository hygiene

- Do not store SCIP indexes, AST dumps, or generated reports in the repository
  unless the project explicitly adopts them as versioned artifacts.
- Prefer temporary directories created with `mktemp -d`.
- Do not modify or reset unrelated working-tree changes during analysis.
- Commit documentation, individual cleanup families, and broader architectural
  refactors separately.
- Treat tool output as evidence to review, not as authorization for automatic
  code deletion.
