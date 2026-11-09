#!/usr/bin/env python3
"""Reject reintroduction of the retired Task/Worker implementation.

Ordinary Session Kernel executions, Subagents and OS/background activities are
intentionally supported. Historical names belong only to the one-time database
migration and its fixtures, never current runtime/API/client code.
"""
from __future__ import annotations
import argparse
from pathlib import Path
import re
import subprocess
import sys

RETIRED_PATHS = (
    'crates/agena-superagent',
    'crates/agena-runtime/src/runtime/superagent.rs',
    'crates/agena-runtime/src/runtime/superagent',
    'crates/agena-storage-sqlite/src/superagent_repository.rs',
    'crates/agena-api/src/tasks.rs',
    'crates/agena-api-server/src/rest/superagent.rs',
    'crates/agena-client/src/http/tasks.rs',
    'crates/agena-plugin-sdk/src/host_api/work.rs',
    'crates/agena-bundled-plugins/src/plugins/provided/work.rs',
    'crates/agena-bundled-plugins/assets/prompts/conversation-work.md',
    'crates/agena-tui-app/src/app_tasks.rs',
    'packages/agena-web/src/pages/TasksPage.vue',
    'packages/agena-web/src/features/tasks',
    'packages/agena-web/src/lib/taskApi.ts',
    'docs/superagent.md',
    'docs/conversation-work.md',
)
RULES = {
    'retired Task domain': re.compile(r'\b(?:TaskRepository|SqliteTaskRepository|CreateTask|ResumeTask|TaskQuery|TaskState|ConversationWork|WorkNotice|WorkRequest|WorkAction|AutomationTask)\b'),
    'retired worker lifecycle': re.compile(r'\b(?:ensure_task_worker|worker_session_id|conversation_work_context|conversation_operation|deliver_conversation_work|require_conversation_work_user)\b'),
    'retired plugin or callback': re.compile(r'\b(?:agena\.work|work\.(?:manage|sessions|connections)|conversation_work)\b'),
    'retired HTTP surface': re.compile(r'/api/v1/(?:tasks(?:[/"\x27?]|$)|chatbot/messages)'),
    'retired channel/automation field': re.compile(r'\b(?:background_work|max_task_seconds)\b'),
    'retired crate/service': re.compile(r'\b(?:agena[-_]superagent|SuperagentService|SuperagentRuntime)\b'),
}
# These files decode the exact former schema once. No active repository or
# executor may be added here; migration behavior has executable storage tests.
MIGRATION_ROOTS = ('crates/agena-storage-sqlite/src/kernel/retirement/',)
MIGRATION_FILES = set()
SOURCE_ROOTS = ('crates/', 'packages/agena-web/src/', 'tools/')
EXTENSIONS = {'.rs', '.ts', '.vue', '.json', '.toml', '.md'}


def is_current_source(relative: str) -> bool:
    if not relative.startswith(SOURCE_ROOTS):
        return False
    path = Path(relative)
    if path.suffix not in EXTENSIONS:
        return False
    if any(part in {'tests', 'fixtures', 'generated'} for part in path.parts):
        # Generated tool manifests are checked separately below.
        return False
    if path.stem.endswith(('_test', '_tests', '.test', '.spec')) or path.stem == 'tests':
        return False
    return relative not in MIGRATION_FILES and not relative.startswith(MIGRATION_ROOTS)


def audit(root: Path, paths: list[str]) -> list[str]:
    failures = [f'retired path still exists: {path}' for path in RETIRED_PATHS if (root / path).exists()]
    generated = root / 'crates/agena-bundled-plugins/generated/tools-reference.md'
    if not generated.is_file():
        failures.append('generated tool contract is missing')
    else:
        contract = generated.read_text(encoding='utf-8')
        for tool in ('bot.sessions', 'bot.connections', 'agent.spawn', 'companion.manage'):
            if tool not in contract:
                failures.append(f'required native capability missing from generated contract: {tool}')
    for relative in paths:
        generated_tools = relative.startswith('crates/agena-bundled-plugins/generated/') or relative == 'packages/agena-web/src/generated/agenaState.ts'
        if not (is_current_source(relative) or generated_tools):
            continue
        file = root / relative
        if not file.is_file():
            continue
        source = file.read_text(encoding='utf-8')
        if file.suffix == '.rs':
            # Negative assertions may name retired tools. Ignore only complete
            # top-level test modules, retaining line numbers and later code.
            source = re.sub(
                r'(?ms)^#\[cfg\(test\)\]\s*\n(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{.*?^\}',
                lambda match: '\n' * match.group(0).count('\n'),
                source,
            )
        for number, line in enumerate(source.splitlines(), 1):
            for label, rule in RULES.items():
                if rule.search(line):
                    failures.append(f'{relative}:{number}: {label}')
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    root = args.root.resolve()
    result = subprocess.run(['git', '-C', str(root), 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], check=True, capture_output=True)
    paths = sorted(set(filter(None, result.stdout.decode().split('\0'))))
    failures = audit(root, paths)
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print('Task retirement audit passed: no retired implementation, tools, API or UI source remains.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
