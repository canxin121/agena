# Agena patch for `proc-macro-error2` 2.0.1

This directory contains the library source published as `proc-macro-error2`
2.0.1 under the MIT or Apache-2.0 license:

- Upstream repository: <https://github.com/GnomedDev/proc-macro-error-2>
- crates.io package checksum:
  `11ec05c52be0a07b08061f7dd003e7d7092e0472bc731b4af7bb1ef876109802`

Agena carries one future-compatibility patch: the crate's `proc_macro` extern
crate declaration is public because the library publicly re-exports it from
its hidden export module. Rust's `pub_use_of_private_extern_crate` transition
currently reports the upstream declaration as a future incompatibility and
will reject it in a future compiler release.

The patch is the compiler-recommended one-token visibility change. Remove this
vendored override after an upstream release contains the same fix. The local
manifest contains only the published library dependencies and features; the
upstream tests, development dependencies, and project-specific lint policy are
not part of Agena's dependency build. A local Clippy allowance preserves the
published documentation verbatim despite a newer list-indentation style lint.
