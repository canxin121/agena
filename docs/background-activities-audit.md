# Background Activities TUI audit

Date: 2026-08-11

## What the project currently exposes

There are two different interfaces that can look like “background activities”:

1. Transcript activity folding displays text such as “older activity blocks folded”. This only compacts prior tool/thinking rows in the conversation. It is not the Background Activities panel and does not represent process state.
2. The dedicated panel is opened through `/activities`, `/background`, or `/tasks`. Its implementation is in `crates/agena-tui/src/activities.rs`, with application integration in `crates/agena-tui-app/src/app_activities.rs` and `crates/agena-tui-app/src/view/view_overlays/view_activities.rs`.

The dedicated panel currently supports:

- active and finished records;
- pending, running, succeeded, failed, cancelled, and stopped lifecycle states;
- shell, task, runtime, and browser activity kinds;
- title, kind, state, duration, identifiers, descriptions, and detail/log data;
- keyboard navigation with arrows, Page Up/Down, Ctrl-B/Ctrl-F, Enter, refresh, stop, dismiss, filter, kill, and escape actions.

## Visual and interaction problems

- The list permanently consumes 58% of the width even when the detail pane is closed, leaving a large visually empty region.
- The footer advertises `q` to close, but the event handler closes with Escape.
- Filter state and available filter values are not clearly surfaced, so users cannot tell why records disappeared.
- Selection uses the unfiltered row index in several actions. After filtering, stop/dismiss/detail can target a different activity from the highlighted row.
- Moving selection does not consistently reload detail logs for the newly selected activity.
- The panel does not automatically refresh while an activity is running, making a healthy process look frozen.
- Wrapped rows occupy multiple terminal lines, but scroll calculations treat each activity as one line. The highlight and viewport drift on narrow terminals or long titles.
- Log presentation is snapshot-oriented and reverse-biased rather than an obvious live tail with a stable cursor.
- Status filtering does not consistently include cancelled and stopped terminal states.
- Current foreground session execution is not projected into this registry, so the panel is not a complete explanation of why a session is busy.

## Relationship to the reported hangs

The session 25 hang was a session-state livelock. The session 26 hang was an unbounded broad `fs.grep`. Neither was caused by this panel's renderer. However, the panel's lack of live refresh and incomplete activity coverage made both incidents harder to diagnose because it could not show a trustworthy “what is actually running now” view.

## Implemented remediation

The dedicated panel was corrected after this audit:

- request IDs are now written directly to the temporarily detached route state, fixing reload and log responses that were always discarded as stale during key handling;
- selection is resolved through the filtered projection for detail, stop, and dismiss actions;
- list and log polling have separate in-flight gates, cadences, request IDs, and ten-second deadlines;
- stop/dismiss/clear mutations have their own gate and monotonic request ID, so a list response or a response from a previously closed panel cannot unlock or mutate a newer operation;
- the footer summary reserves its poll window before spawning, preventing one request per 50 ms UI tick while the first request is pending;
- active logs use a stable sequence cursor, append chronologically, retain at most 500 lines, and distinguish waiting, no output, truncation, and read errors;
- the list uses the full width while detail is closed, splits horizontally on wide terminals, and stacks vertically on narrow terminals;
- rows are constrained to one rendered line so keyboard selection and scroll offsets cannot drift;
- the command bar exposes the actual `Esc` close binding first, shows only legal row actions, and keeps active filters in the title;
- cancelled and stopped are now part of the status-filter cycle.

Renderer, responsive-layout, filtered-selection, log-order, and bounded-log-merge behavior are covered by targeted tests. Foreground session/tool execution is still not projected into this registry; that is a model-coverage enhancement rather than a panel liveness defect.

## Original redesign order

1. Fix filtered-index identity and action targeting before visual changes.
2. Add event-driven refresh and stable log cursors; show last-output time and explicit “waiting / no new output” state.
3. Use a full-width list when detail is closed and a responsive split only after Enter opens detail.
4. Replace the overloaded footer with a compact command bar that reflects the current row's legal actions.
5. Surface active filters as chips/tags and include cancelled/stopped states.
6. Make row height part of viewport calculation, or constrain each row to one line with a detail preview elsewhere.
7. Project foreground session/tool execution into the same activity model so the panel can answer why the application is busy.
