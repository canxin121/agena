# Agena dependency patch

Tantivy 0.26.1 requires `lru ^0.16.3`. Agena raises that requirement to
`^0.18.2`, which fixes the panic-safety unsoundness in `LruCache::pop`
([RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html)).
This also removes the duplicate LRU version from the application graph.

Tantivy only uses LRU in `src/store/reader.rs`: `new`, `get`, `put`, `len`,
and `peek_lru` in tests. Those APIs are unchanged. The intervening versions
update hashbrown and fix an unrelated method's lifetime; the new Rust 1.85
minimum remains below Agena's toolchain requirement.

Keep `Cargo.toml` and `Cargo.toml.orig` in sync. Remove this dependency patch
when the selected upstream Tantivy version requires a fixed LRU release.
Validate with the memory-index tests and the workspace suite.
