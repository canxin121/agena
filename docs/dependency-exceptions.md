# Dependency exceptions

Last reviewed: 2026-07-13
Next review: 2026-10-13
Owner: Agena maintainers

This document records dependency findings that cannot currently be removed by
changing only Agena-owned code while preserving the corresponding feature. A
listed item is not a blanket waiver: new vulnerabilities, direct unmaintained
dependencies, and newly yanked packages still fail the repository checks.

## Enforced policy

- Known vulnerabilities and unsound advisories are checked across the complete
  resolved dependency graph.
- Unmaintained advisories fail `cargo-deny` when the affected crate is a direct
  dependency of a workspace package. Transitive unmaintained advisories remain
  visible in the scheduled report and are reviewed here.
- An ignored advisory must be present in this document, include a removal
  condition, and be removed from `deny.toml` as soon as the condition is met.
- JavaScript installs used for builds are frozen to the checked-in lockfile.
- GitHub Actions and the experimental CEF Tauri CLI are pinned to immutable
  commits.

## Security exceptions

| Advisory | Resolved package and path | Exposure and mitigation | Removal condition |
| --- | --- | --- | --- |
| `RUSTSEC-2023-0071` | `rsa 0.9.10`, present only in the lockfile through optional `sqlx-mysql` | Agena enables only SQLx SQLite; `cargo tree -i rsa` has no active path. `cargo-audit` needs an explicit ignore because it scans every lockfile record. | Remove when SQLx no longer resolves the affected RSA release, or immediately if the MySQL feature is enabled. |
| `RUSTSEC-2024-0429` | `glib 0.18.5` through Tauri/Wry's Linux GTK3 backend | The advisory concerns `VariantStrIter`; Agena does not call GLib or that API directly. Linux desktop builds still require the upstream GTK3 stack. This is the sole `cargo-deny` advisory ignore. | Remove when Tauri/Wry moves to GLib 0.20 or newer, or when Agena no longer ships the GTK3 desktop backend. |

## Transitive unmaintained packages

These are warnings rather than security vulnerabilities. They are not ignored
individually in `deny.toml`; the `unmaintained = "workspace"` policy keeps them
visible without making an upstream-only migration block every change.

| Advisory IDs | Upstream dependency path | Current mitigation and removal condition |
| --- | --- | --- |
| `RUSTSEC-2024-0411` through `RUSTSEC-2024-0420` (GTK3 set) and `RUSTSEC-2024-0370` | Tauri/Wry -> GTK3/GLib | Linux-only desktop stack; no direct Agena GTK API usage. Remove when Tauri adopts a maintained Linux GUI stack. |
| `RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`, `RUSTSEC-2025-0081`, `RUSTSEC-2025-0098`, `RUSTSEC-2025-0100` | Tauri Utils -> `urlpattern 0.3` -> `unic-*` | URL pattern parsing is upstream Tauri code. Remove when Tauri upgrades `urlpattern` to a maintained Unicode implementation. |
| `RUSTSEC-2025-0141` | CLI -> `syntect 5.3` -> `bincode 1.3` | Syntect's embedded default syntaxes and themes require its bincode dump format. YAML loading was disabled, so `yaml-rust` is no longer resolved. Remove by replacing Syntect or when Syntect changes its embedded-data format. |
| `RUSTSEC-2026-0173` | `merge 0.2 -> merge_derive` and `sea-orm 1.1 -> sea-bae` -> `proc-macro-error2` | Compile-time procedural macros only. Remove by replacing Agena's `merge` derives and migrating SeaORM call sites, or when those upstream crates stop using it. |
| `RUSTSEC-2024-0014`, `RUSTSEC-2024-0436` | Plugin ABI -> `abi_stable 0.11` -> `generational-arena`/`paste` | The packages support the native plugin ABI; plugin inputs still pass Agena validation and permission checks. Remove when `abi_stable` replaces them or when the native ABI is retired. |
| `RUSTSEC-2026-0192`, `RUSTSEC-2026-0206` | CLI SVG/math rendering -> `resvg`/vendored RaTeX -> `ttf-parser`/`rustybuzz` | Rendering is local and receives bounded inputs. Remove when resvg and RaTeX migrate to maintained font libraries, or when the renderer is replaced. |

## Deliberate compatibility holds

| Package | Hold | Removal condition |
| --- | --- | --- |
| Web TypeScript | `~6.0.3` instead of TypeScript 7 | `vue-tsc 3.3.7` still loads `typescript/lib/tsc`, which TypeScript 7 does not publish. Upgrade after Vue language tools officially support the TypeScript 7 native compiler API and the full Vue SFC typecheck passes. |
| Experimental CEF Tauri CLI | Commit `3b2823b918d5ea88fca10b472daf349c67c22d51` from upstream `feat/cef` | This is pinned because the feature has no stable release. Update intentionally after the CEF desktop build passes, or replace it with a stable Tauri release. |

## Upstream exact transitive pins

`cargo update --workspace --verbose` reports five older patch releases. Each is
constrained by an exact version requirement in an already-current direct
dependency, so Agena cannot update it from the workspace manifest:

| Locked package | Newer release in the same series | Exact upstream constraint |
| --- | --- | --- |
| `generic-array 0.14.7` | 0.14.9 | `crypto-common 0.1.7` requires `=0.14.7`. |
| `matchit 0.8.4` | 0.8.6 | `axum 0.8.9` requires `=0.8.4`. |
| `toml 0.8.2` | 0.8.23 | GTK3's `system-deps 6.2.2` and the old GLib macro stack resolve a mutually pinned TOML set. |
| `toml_datetime 0.6.3` | 0.6.11 | `proc-macro-crate 2.0.2` requires `=0.6.3`. |
| `toml_edit 0.20.2` | 0.20.7 | `proc-macro-crate 2.0.2` requires `=0.20.2`. |

Remove these entries when `crypto-common`, Axum, or Tauri's GTK dependency
stack releases versions with relaxed or newer constraints. Do not patch Cargo's
registry sources or fork those packages solely to change a transitive pin.

## Vendored inventory

Vendored sources are intentionally excluded from automated upgrades, but they
remain part of the review inventory.

| Directory | Version | Files | Approximate size | Coupled workspace packages |
| --- | ---: | ---: | ---: | --- |
| `third_party/ratatui-image` | 11.0.6 | 20 | 220 KiB | `ratatui-image` |
| `third_party/ratex-unicode-font` | 0.1.13 | 240 | 68 MiB | `ratex-layout`, `ratex-parser`, `ratex-render`, `ratex-types` |

Total: 2 vendored directories, 260 files, approximately 68 MiB. Their five
coupled Cargo packages are excluded from `cargo upgrade` reports until the
vendored sources are upgraded as one tested unit.
