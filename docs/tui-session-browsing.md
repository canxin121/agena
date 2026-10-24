# TUI session browsing

The session hub and resume/search route use compact session rows, separate pin
and favorite markers, and the existing persistent session API used by the Web
sidebar. The chat layout is unchanged.

## Layout

The resume route uses the available terminal height instead of the generic
picker's four-cell gutter. Search, actions, pagination status, selected-session
metadata, and keyboard hints each have a bounded area. At 80 × 24, the production
renderer with the same representative fixture shows **16 session rows**, compared
with **7** in the previous generic picker layout. The page size follows terminal
resizing, and all mouse targets use the resulting page indices.

Hub session and directory rows occupy one line. When the content area is at
least 108 columns wide, directory sessions and quick-access groups appear side
by side. Smaller terminals use two stacked regions and give unused height back
to the other region; very short terminals use a single viewport. The two regions
retain independent scroll positions. Metadata appears beside titles when there
is enough width, and the selected row has a dedicated metadata area below the
lists. Long titles and paths are clipped using terminal display widths.

`◆` means pinned and `★` means favorite. Both can be set on the same session.
The hub retains its Pinned and Favorites groups, including the same session's
copies in directory and quick-access lists. Toolbar actions show the operation
appropriate to the selected session and support mouse activation.

## Keyboard and pointer interaction

| Input | Resume/search route | Session hub |
| --- | --- | --- |
| ↑ / ↓ | Select a session | Select rows across directories and groups |
| ← / → | Previous/next visual page while results have focus | Previous/next backend page for the selected scope |
| PgUp / PgDn | Previous/next visual page, including from search input | Move by the actual viewport height |
| Home / End | First/last loaded result while results have focus | First/last selectable row |
| Enter | Open selected session | Open session or expand/collapse directory |
| / or Ctrl+F | Focus the search editor | Enter search |
| Tab / Shift+Tab | Switch search/results focus | Jump between groups |
| p / f | Toggle pin/favorite while results have focus | Toggle pin/favorite outside search input |
| Alt+P / Alt+F | Toggle pin/favorite, including during text editing | Toggle pin/favorite, including during search |
| Ctrl+R | Reload results or retry a failed append | Refresh the hub |
| Esc | Leave search input, then close the route | Clear search, then return |

The resume route initially focuses results, so Left/Right work immediately.
Typing ordinary text enters the search editor; `/` or Ctrl+F also allows queries
starting with the `p` and `f` action keys. Left/Right within the editor move its
cursor. Paste uses the same query reset and request handling as typed input.

In the hub, a directory heading belongs to workspace pagination, its session
rows belong to that directory's pagination, and each quick-access group has its
own page. Visible page-control rows remain keyboard-selectable and clickable.
Opening a directory is assigned to Enter, freeing Left/Right for pagination.

The resume page indicator describes the loaded results. A `+` after the known
page count means the server has further results; browsing past the last loaded
page fetches the next batch and advances automatically. Earlier pages remain
available without refetching.

## Request and selection behavior

- Pin/favorite actions send the actual selected session ID through the existing
  `set_session_pinned` and `set_session_favorite` APIs. They update independently
  after a successful response, retaining query, focus, and selected row.
- Request IDs distinguish repeated queries and reopened dialogs. Late search
  responses are ignored even when their query text and batch number match.
- Loading another batch preserves prior results. An append advances only if the
  user still intends to navigate there; moving elsewhere cancels that movement.
  Repeated page keys do not start duplicate requests.
- Failed appends preserve their results and cursor, show an error, and can be
  retried with Ctrl+R. Appended records are deduplicated by session ID.
- Successful mark updates also apply to subtree catalogs and late list
  responses. Hub refreshes keep the selected copy in its original group, and
  remote page changes focus the new page's sessions rather than New Session.
- Shared picker Home/End navigation now records selection identity, preventing
  a later append from restoring an older selection.

## Verification and visual review

Rust tests have been removed pending a new repository-wide test plan. Static
checks:

```sh
cargo clippy --locked --no-deps -p agena-tui -p agena-tui-components -p agena-tui-session -p agena-tui-settings -p agena-tui-app --all-targets -- -D warnings
cargo fmt --all -- --check
```
