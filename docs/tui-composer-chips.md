# TUI composer border chips

The TUI chat surface has exactly two rows: the transcript and the composer.
Nothing docks between them. Everything the session wants to surface rides on
the composer's own border rows, matching the web composer (`Composer.vue`
status / bottomLeft slots).

## Border layout

| Corner | Content | Click |
| --- | --- | --- |
| Top-left | Session status: model, thinking mode, speed mode, context usage | Model chooser, thinking/speed overlays, usage dashboard |
| Top-right | Prompt-history search, pending approvals | History palette, pending approval overlay |
| Bottom-left | One segment per session surface: recorded file edits (`±n files`), background tasks (`●n tasks`), BTW questions (`BTW n`) | Opens the matching full-screen surface |
| Bottom-right | Plan progress contributed by the `agena.plan` plugin (`▶ 2/5 ↻`) | Opens the full-screen plan viewer |

The bottom-left chip is a single string joined with ` | `; each segment keeps
its own pointer target, so a narrow terminal that truncates the chip still
routes clicks to the visible segments.

## Full-screen surfaces

| Surface | Route | Opened by |
| --- | --- | --- |
| Plan viewer | `Route::PlanViewer` | Plan chip, `/plan`, `Ctrl+P` |
| Session work | `Route::SessionWork(id)` | `±n files` / `●n tasks` segments, `Ctrl+]` |
| BTW question | `Route::Btw(id)` | `BTW n` segment, `/btw` |

`Esc`/`Tab` (and `q` in the session work surface) return to the chat composer.
Opening another session closes any of these surfaces because they belong to the
session that opened them.

## Why

The web client shows plan progress, workspace changes, background tasks and
BTW questions as chips on the composer border. The TUI previously reserved up
to three extra rows above the composer for the same information, which cost
transcript space and drifted from the web layout. The border chips are now the
only presentation; the details live in full-screen routes instead of docked
panels.

## Data sources

- Plan progress: the `agena.plan` status-line display contribution (`plan:{session}`).
  Opening a session re-reads the plan once so the chip survives a restart.
- Files: the session file-change summary (polled while the chat surface is visible).
- Tasks: the session execution's background activities.
- BTW: the in-memory BTW exchange list for the current session.
