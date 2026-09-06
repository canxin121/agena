# Agena build patch

Ordinary builds, including builds with `dynamic-versions`, copy the committed
`chrome_versions.rs.fallback` into Cargo's `OUT_DIR`. They do not fetch Chrome
release metadata or rewrite the source file. The upstream build fetched live
JSON and overwrote this tracked file, making `--locked` builds depend on the
network, dirtying checkouts, and producing different binaries from one commit.

Maintainers can explicitly set `SPIDER_FP_REFRESH_CHROME=1` for a corpus refresh
and review the resulting diff before committing it. The existing optional
referrer regeneration switches remain explicit maintenance actions.

The fallback copy now fails with a useful error if its input is missing, and
Cargo watches the actual fallback path instead of a nonexistent `build/` path.
