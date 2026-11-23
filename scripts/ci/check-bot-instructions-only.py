#!/usr/bin/env python3
"""Fail CI if removed per-Bot personality options reappear.

AssistantPreferences owns memory and reminder policy. Named BotPersona owns
additional system instructions and prompt references.
"""
from __future__ import annotations
from pathlib import Path
import re
import sys

root=Path(__file__).resolve().parents[2]
errors=[]

def check(path:str,required:tuple[str,...]=(),forbidden:tuple[str,...]=()):
    file=root/path
    if not file.is_file():errors.append('Missing '+path);return
    text=file.read_text()
    for term in required:
        if not re.search(term,text,re.S):errors.append('Missing '+repr(term)+' in '+path)
    for term in forbidden:
        if re.search(term,text,re.S):errors.append('Legacy Bot persona field '+repr(term)+' in '+path)

check('crates/agena-assistant/src/bot.rs',
      (r'pub struct BotPersona\s*\{\s*pub instructions: String,.*?pub prompts: Option<BotPromptSelection>,\s*\}',
       r'\#\[serde\(deny_unknown_fields\)\]\s*pub struct BotPersona',
       r'if self\.instructions\.len\(\) > MAX_BOT_INSTRUCTIONS_BYTES'),
      (r'AssistantPreferences', r'AssistantTone', r'address_user:', r'\blanguage:'))
check('packages/agena-web/src/generated/agenaState.ts',
      (r'export type BotPersona\s*=\s*\{\s*instructions: string\s*prompts\?: BotPromptSelection\s*\}',))
check('packages/agena-web/src/lib/bots.ts',
      (r'emptyBotPersona\s*=.*\{\s*instructions:',),
      (r'address_user:',r'\btone:',r'\blanguage:'))
for file in ('packages/agena-web/src/features/bots/BotManagerHost.vue',
             'packages/agena-web/src/layout/BotSidebar.vue',
             'crates/agena-tui-app/src/app_bot.rs',
             'crates/agena-tui-app/src/app_bot/render.rs'):
    check(file,forbidden=(r'persona\.address_user',r'persona\.tone',r'persona\.language',
                          r'bot-edit-address',r'bot-edit-tone',r'bot-edit-language'))
check('crates/agena-runtime-session/src/session/manager/bot/registry.rs',
      (r'profile\.persona\.instructions', r'pub async fn bot_profile_operation'))
check('crates/agena-runtime/src/runtime/assistant/companion.rs',
      forbidden=(r'\bBotUpdate\b', r'\bbot\.persona\.'))
check('crates/agena-assistant/src/companion.rs',
      forbidden=(r'pub struct Assistant(?:Preferences|Patch)\s*\{[^}]*\bpub (?:name|address_user|tone|language|instructions):',))
if errors:
    print('\n'.join(errors),file=sys.stderr)
    raise SystemExit(1)
print('Bot persona contract audit passed: bounded instructions and saved prompt references.')
