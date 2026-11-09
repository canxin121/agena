#!/usr/bin/env python3
"""Reject reintroducing default-project scope or historical migration in global Bots.

The external, one-time database conversion tool intentionally lives outside
this source tree. Runtime creation, readers and sidebars only accept current data.
"""
from __future__ import annotations

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
REQUIRED = {
    'crates/agena-storage-sqlite/src/kernel/schema.rs': [
        r'workspace_id INTEGER REFERENCES agena_workspaces\(id\)',
    ],
    'crates/agena-runtime-session/src/session/manager/bot/registry.rs': [
        r'create_global\(',
        r'deny_unknown_fields',
    ],
    'crates/agena-runtime-contracts/src/lib.rs': [r'fn is_global\('],
    'crates/agena-runtime/src/runtime/snapshot/mod.rs': [
        r'global_host_client:',
        r'global_plugins:',
        r'global_mcp_manager',
        r'global_plugin_config:',
    ],
    'crates/agena-runtime/src/runtime/builder.rs': [
        r'global_host_client\.bind_runtime\(',
        r'install_global_plugin_host\(',
    ],
    'crates/agena-runtime-plugins/src/plugin_slot.rs': [
        r'fn install_global_plugin_host\(',
        r'current_host_callback_context\(',
    ],
    'packages/agena-web/src/layout/BotSidebar.vue': [r'SidebarBots'],
    'packages/agena-web/src/layout/MainLayout.vue': [
        r'<BotSidebar v-if="route.path === \'/bot\'"',
    ],
    'packages/agena-web/src/layout/AppDesktopSidebar.vue': [r'activeTab === \'bot\''],
}
FORBIDDEN = {
    'crates/agena-storage-sqlite/src/kernel/schema.rs': [
        r'(?<!test_)migrate_nullable\(',
        r'migrate_retired_prompt',
        r'retirement::migrate',
        r'retirement::is_predecessor',
        r'#[^\n]*path\s*=\s*"retirement/',
    ],
    'crates/agena-runtime-session/src/session/manager/bot/registry.rs': [
        r'operator_bot_v1',
        r'operator_bot_seed_v1',
        r'migrate_bot_roots\(',
        r'detach_bot_root\(',
        r'current_workspace_id\(',
        r'\bglobal_scope\b',
        r'\bworkspaces\s*:',
    ],
    'crates/agena-runtime-session/src/session/manager/bot.rs': [
        r'const BOT_IDENTITY\b',
        r'const BOT_SEED\b',
        r'operator_bot_v1',
        r'operator_bot_seed_v1',
    ],
    'packages/agena-web/src/layout/BotSidebar.vue': [
        r'<ChatSidebar',
        r'<ChatSidebarHeader',
        r'PinnedSessions',
        r'RecentSessions',
        r'DirectorySession',
    ],
}
ABSENT = [
    'crates/agena-storage-sqlite/src/kernel/retirement',
]


def audit(root: Path) -> list[str]:
    errors = []
    for relative in ABSENT:
        if (root / relative).exists():
            errors.append(f'obsolete migration implementation retained: {relative}')
    for relative, expressions in REQUIRED.items():
        source = root / relative
        if not source.is_file():
            errors.append(f'missing global-scope implementation: {relative}')
            continue
        text = source.read_text()
        for expression in expressions:
            if not re.search(expression, text):
                errors.append(f'missing {expression!r} in {relative}')
    for relative, expressions in FORBIDDEN.items():
        source = root / relative
        if not source.is_file():
            continue
        text = source.read_text()
        # Test cases are free to assert historical payloads are rejected.
        text = text.split('#[cfg(test)]', 1)[0]
        for expression in expressions:
            if re.search(expression, text):
                errors.append(f'retired runtime fallback {expression!r} in {relative}')
    return errors


def main() -> int:
    errors = audit(ROOT)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    print('Global Bot scope audit passed: no project fallback, old registry or runtime migrations.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
