#!/usr/bin/env bash
# Remove agena's iTerm2 Option-as-Alt takeover setup.
#
# Deletes the "Agena Option Alt" dynamic profile and restores the OSC 1337
# SetProfile advanced setting to its default (prompt on first use) when it
# was exactly the value this setup installed.
set -euo pipefail

FILE="$HOME/Library/Application Support/iTerm2/DynamicProfiles/agena-option-alt.json"

if [ -f "$FILE" ]; then
  rm -f "$FILE"
  echo "Removed $FILE"
else
  echo "No dynamic profile found; nothing to remove."
fi

if defaults read com.googlecode.iterm2 PreventEscapeSequenceFromChangingProfile 2>/dev/null | grep -qx '0'; then
  defaults delete com.googlecode.iterm2 PreventEscapeSequenceFromChangingProfile
  echo "Restored PreventEscapeSequenceFromChangingProfile to its default."
else
  echo "PreventEscapeSequenceFromChangingProfile was not set by this setup; leaving it alone."
fi

echo "Done. iTerm2 hot-reloads dynamic profiles; sessions already on the"
echo "\"Agena Option Alt\" profile stay on it until switched back manually."
