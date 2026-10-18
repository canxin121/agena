# TUI settings

The settings route now uses its own renderer in `agena-tui-settings`. It follows the web settings page's category rail and descriptive setting rows while retaining the existing application-owned editors and persistence operations.

## Layout

- Terminals with at least 80 columns show a grouped category rail and a settings pane. Narrower terminals show one pane at a time; Tab and the arrow keys switch panes.
- Each setting shows its name, effective value, and a short description. Boolean values use a symbol and localized on/off text. Long strings are clipped to the row width; Ctrl+D opens the complete description, value, configuration sources, and path.
- Details have their own scroll position and pointer targets. Esc closes details without leaving settings.
- The theme uses the existing semantic palette, including terminal background detection and light/dark preferences. No chat layout or transcript rendering changes are involved.
- The footer keeps the back/clear action visible at its right edge. Less essential shortcuts disappear as the available width shrinks.

## Interaction

| Key | Action |
| --- | --- |
| Tab / Shift+Tab, ← / → | Switch between categories and settings |
| ↑ / ↓ | Select a category or setting |
| PgUp / PgDn, Home / End | Page through settings or move to either end |
| / or Ctrl+F | Search categories, setting names, descriptions, values, and configuration paths |
| Enter in search | Focus the filtered results without changing a configuration value |
| Enter on a setting | Open its existing editor or action |
| Ctrl+D | Inspect the selected setting |
| Ctrl+R | Refresh the current settings view |
| Esc | Close details, clear a search, or return from settings |

Search supports Unicode, multiple terms, cursor editing, and paste. Categories show matching counts. Mouse targets use the filtered positions, and asynchronous settings rebuilds retain the query and input focus. Categories remember their selected setting while navigating.

## Verification and previews

```sh
cargo test --locked -p agena-tui -p agena-tui-settings --lib
cargo test --locked -p agena-tui-app --lib settings_
cargo test --locked -p agena-tui-app --lib app_pointer::tests
cargo clippy --locked --no-deps -p agena-tui-settings -p agena-tui -p agena-tui-app --all-targets -- -D warnings
cargo fmt --all -- --check
```

These targeted checks pass. Including dependency linting also encounters the
existing `clippy::large_enum_variant` diagnostic in `agena-provider`'s provider
event type; that type is outside this settings change.

The settings tests render the production widget at multiple terminal sizes in all supported locales, including tiny viewports. Integration tests exercise filtered mouse activation, search paste, narrow navigation, and returning from details. A separate, explicitly invoked test writes SVG and text previews from Ratatui's actual cell buffer using representative fixture data:

```sh
AGENA_SETTINGS_PREVIEW_DIR="$PWD/.tmp-artifacts/settings" \
  cargo test --locked -p agena-tui-settings settings_write_previews -- --ignored
```
