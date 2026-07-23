# Agena Architecture V2 — Phase 9 Performance and Build-Graph Plan

**Status:** separate follow-up plan. The V2 functional architecture refactor
is complete and verified in
[`architecture-v2-refactor-plan.md`](architecture-v2-refactor-plan.md).
This document owns only build-graph isolation, cold-start measurement,
rebuild attribution, and timing-policy work. It is intentionally not a
prerequisite for the completed functional refactor.

## Scope and operating rules

This plan may be scheduled only by an explicit performance-work decision. It
must not be used to reopen Runtime/Core ownership, reintroduce a compatibility
facade, or move a concrete implementation merely to improve an intermediate
measurement.

Normal incremental compilation remains a product invariant:

```toml
[profile.dev]
incremental = true

[profile.test]
incremental = true

[profile.dist]
incremental = false
```

`CARGO_INCREMENTAL=0` is permitted only inside an isolated cold sample created
by `scripts/check-build-timings.sh`; it must not leak into normal development,
test, or broad functional gates. Every timing change must preserve this rule.

The final app remains the only default terminal binary. Default builds must not
pull Studio, examples, or E2E tools, and TUI-only work must not recompile
Runtime, concrete provider adapters, SQLite, API server, or the remote client.

## Current evidence and known gap

The repository already has real leaf boundaries:

- `agena-provider-google-auth` owns the `gcp_auth` SDK call and typed ADC
  errors;
- `agena-provider-bedrock-auth`, `agena-provider-bedrock-signing`, and
  `agena-provider-bedrock-streaming` own the AWS credential, SigV4, and Smithy
  decoder leaves;
- `agena-memory-index` owns the Tantivy retrieval algorithm;
- `agena-web` owns its Moka fetch cache and Governor pacing coordinator.

`scripts/check-build-timings.sh` already supplies cache-warm no-change, TUI
leaf, CLI leaf, final-app, isolated-cold, and rebuild-attribution machinery.
The architecture checker protects the timing script's target isolation and TUI
leaf recompile assertions. Historical cache-warm samples met their declared
budgets, but a reproducible current cold-start baseline and an explicit
threshold/CI policy remain open.

Historical measurements are context only, not current proof: a previous report
recorded 0.70s TUI no-change, 1.13s root no-change, 0.95s TUI leaf, 1.71s CLI
leaf, and 7.97s final-app leaf. Re-establish all values on the scheduled
worktree before accepting or changing any threshold.

The retained target budgets are:

| Scenario | Target budget |
| --- | ---: |
| No-change `cargo check -p agena-tui` | <= 1 second |
| TUI leaf change, `cargo check -p agena-tui` | <= 15 seconds |
| CLI leaf change, `cargo check -p agena-cli` | <= 10 seconds |
| TUI leaf change, final `agena` build | <= 30 seconds |
| No-change root `cargo build` | <= 2 seconds |
| Default final terminal link count | exactly 1 |

## Work items

1. **[x] Maintain actual heavy-SDK leaf ownership.** Preserve the named Google,
   Bedrock, memory-index, and web leaves. If a measurement shows an unwanted
   rebuild, trace the manifest and source edge first; do not hide it with a
   wrapper crate, alias, compatibility re-export, or cosmetic feature flag.
2. **[x] Retain real capability feature checks and parallel CI layers.** Keep
   only concrete product capabilities: final-app schema/plugin flags,
   API-server protocol flags, marketplace `server`, and CLI plugin-signing.
   Keep the full workspace gate alongside parallel focused jobs; feature flags
   must not hide a dependency edge.
3. **[ ] Audit target-graph isolation.** Confirm default-member and binary
   boundaries, then inspect the timing script's verbose Cargo attribution for
   TUI, CLI, and final-app edits. Add a guard only for a real product boundary.
4. **[ ] Establish isolated cold baselines.** Run the cold sample with
   `MEASURE_COLD_START=1` using only the script-created temporary target
   directory. Record machine/environment context and retain the raw report.
5. **[ ] Re-establish rebuild attribution.** Run leaf scenarios with
   `MEASURE_LEAF_CHANGES=1`; investigate unexpected recompilation of Runtime,
   provider/AWS/Google leaves, SQLite, API server, or remote client before
   accepting results.
6. **[ ] Decide enforcement policy.** After repeated comparable samples,
   choose report-only versus `ENFORCE_BUILD_TIMING=1` thresholds. Do not waive
   or lower a threshold without a written causal attribution.
7. **[ ] Update CI and documentation.** Keep feature checks capability-based,
   retain the full workspace gate, and document only measured current values.

## Required scheduled verification

Run functional gates before interpreting a performance result if source or
manifest changes were made. The performance-specific scheduled pass is:

```bash
MEASURE_LEAF_CHANGES=1 MEASURE_COLD_START=1 scripts/check-build-timings.sh
git diff --check
```

When a Phase 9 patch changes source, manifests, architecture guards, or timing
scripts, also rerun the locked functional pipeline from format through
workspace tests, E2E, `cargo machete`, and `cargo deny check`.

## Exit criteria

```text
TUI work does not rebuild concrete provider adapters.
API client work does not rebuild Runtime.
Default build does not build Studio, examples, or E2E tools.
Default terminal build links exactly one agena binary.
Cold-start and leaf-rebuild evidence is reproducible and has an explicit CI
reporting/enforcement policy.
```
