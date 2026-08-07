#!/usr/bin/env bash
# One-time iTerm2 setup for agena's per-session Option-as-Alt takeover.
#
# Creates a dynamic profile "Agena Option Alt" (a child of the Default
# profile) with "Option Key Sends = Esc+", and pre-approves the OSC 1337
# SetProfile control sequence. When agena starts inside iTerm2 it switches
# its own session to that profile via `OSC 1337;SetProfile=...`, so
# Option+Backspace is reported as ALT+Backspace (CSI u 127;3u) and word-
# deletes. On exit agena restores the Default profile, so other sessions
# and other TUI programs are unaffected.
#
# The profile name and GUID here MUST match the constants in
# crates/agena-tui/src/terminal_lifecycle.rs (ITERM2_OPTION_ALT_PROFILE).
#
# Idempotent: safe to re-run. To undo, run scripts/remove-iterm2-option-alt.sh.
set -euo pipefail

PROFILE_NAME="Agena Option Alt"
GUID="A7E0C9F2-1B4D-4C8E-9F3A-6B2D5E8A1C4D"
DIR="$HOME/Library/Application Support/iTerm2/DynamicProfiles"
FILE="$DIR/agena-option-alt.json"

mkdir -p "$DIR"

cat > "$FILE" <<JSON
{
  "Profiles": [
    {
      "Name": "$PROFILE_NAME",
      "Guid": "$GUID",
      "Dynamic Profile Parent Name": "Default",
      "Option Key Sends": 2,
      "Right Option Key Sends": 2
    }
  ]
}
JSON

# Pre-approve the OSC 1337 SetProfile control sequence so agena can switch
# profiles silently (without the one-time "Allow this in the future?"
# prompt). This is the same setting the prompt itself writes.
defaults write com.googlecode.iterm2 PreventEscapeSequenceFromChangingProfile -bool NO

cat <<'EOF'
Installed the "Agena Option Alt" dynamic profile and allowed OSC 1337
profile switching in iTerm2.

- iTerm2 hot-reloads dynamic profiles, so no restart is needed.
- Start agena inside iTerm2: its session now uses "Option Key Sends = Esc+"
  for the duration of the session, so Option+Delete word-deletes. On exit the
  session returns to the Default profile.
- To undo: scripts/remove-iterm2-option-alt.sh
EOF
