# Inline UI learning and repair: deterministic acceptance record

2026-10-28 · main repository `agena` · tests use the real Rust document/program
validator, the Web evaluator, and native TUI transcript renderer. This is **not**
a cold-start language-model evaluation: no real model was run against this guide.
Tool counts below describe the **minimum plugin calls in the verified contract**,
not measured model behaviour or model success rates.

## Guide budget (UTF-8 bytes, not tokens)

| Material | Before | After |
| --- | ---: | ---: |
| Core overview alone | 3,895 | — |
| Five separate basic topics: overview + layout + state + actions + live | 12,756 total across five calls | **15,429** in one default `guide()` call |
| Characters in the complete default response | not measured | 15,405 |
| 45-node dashboard JSON (optional example) | 26,095, plus separately returned schema | **not in default guide** |
| `guide({sections:["chart","expressions","chart"]})` | no batched equivalent | 4,858 (chart included once) |
| `guide({sections:["schema"]})` | not separately measured | 14,562 |
| `guide({sections:["dashboard"]})` | no separate section | 26,150 with heading/wrapping |

The default output is **2,673 bytes larger** than the five old basic Markdown
topics combined, because it includes a fully validated seven-node document, a
minimal update, component property whitelist, and exact expression syntax.
The relevant improvement is the removal of the *multi-call dependency chain*;
this does not establish reduced token usage or higher model success rates.
The default deliberately omits the large dashboard and full schema.

## Deterministic workflow acceptance (plugin calls only)

| Scenario | Calls defined by current protocol | Extra guide sections | Error corrections | Evidence |
| --- | --- | ---: | ---: | --- |
| First slider → computed metric → conditional callout → reset | `guide` → `prepare` (2), then embed | 0 | 0 | Bundled plugin's seven-node fixture, Rust program render/input/activate, Web unit test, native TUI |
| Known contract, create again | `prepare` (1), then embed | 0 | 0 | Preparation requires no guide handshake |
| Select filter drives chart **and** table | `guide` → `prepare` (2) for a first-time caller | 0 | 0 | Existing 45-node fixture's Rust/Web filter tests; no separate model call for local input |
| New agent data for an existing panel | `update` (1) with returned revision | 0 | 0 | `quick-start-update.json`, in-place head update, user state preserved |
| Invalid creation then corrected input | failing `prepare` → corrected `prepare` (2) | 0 | 1 | Located unsupported property, no head after failure, retry succeeds |
| Concurrent revision changed | failed `update` → `read` → reconciled `update` (3) | 0 | 1 reconciliation | CAS conflict exposes expected/current, prior snapshot unchanged, read and deliberate merge |
| Finished at creation / continued work | `prepare` alone creates revision 1 in `complete` / explicit `working`; phase-only update when actually finished | 0 | 0 | Atomic phase persistence and same-reference revision transitions |

The first-time call counts are **contract targets**, not actual experimental
traces from a model. A normal serial update requires no `read`; on conflict,
the read is essential to semantic reconciliation, not just changing a number.
The host preserves compact, bounded model-visible paths and hints across the
plugin error → tool failure → persisted failure → model-message projection.

## Safety and regression boundaries

Untrusted documents cannot allocate a reference before passing internal
validation. Failed updates do not alter the previous head/version or broadcast
a new version; a storage I/O failure is not reported as guaranteed unapplied.
Local input/state and actions remain atomic, and local task checkboxes never
represent completed Agent work. Tests cover session scoping, native TUI widths,
focus and interaction regressions, copy/export, Web/TUI parity and live updates.

The browser scripts `probe-inline-ui.cjs`, `probe-stateful-ui.cjs`, and
`probe-inline-ui-layout.cjs` require the optional Playwright module.
Attempts in this environment failed with `Cannot find module 'playwright'`;
passing the Bun unit suite is not a substitute for executed browser probes.

## Cold-start model evaluation

**Not executed.** No supported real-model evaluation workflow was run for
these scenarios. This record does not claim improved first-try model success
rates, token efficiency, or fewer retries observed from a model.
