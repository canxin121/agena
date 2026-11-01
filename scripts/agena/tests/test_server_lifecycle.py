"""Real HTTP crash recovery and isolated Linux service-manager lifecycle tests.

Run with AGENA_TEST_BINARY pointing to a freshly built Agena executable. The
fixture replaces systemctl in PATH and writes definitions only beneath a
temporary XDG_CONFIG_HOME; the machine's actual user services are never used.
"""

import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener


# Share the established loopback Provider and HTTP fixtures with the probes.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'probes'))
import http_experience as http_fixture


SERVICE_MANAGER = r'''
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import time

root = Path(os.environ['AGENA_TEST_SERVICE_ROOT'])
arguments = sys.argv[1:]
assert arguments.pop(0) == '--user'
with (root / 'manager-calls.jsonl').open('a') as output:
    output.write(json.dumps(arguments) + '\n')
action = arguments[0]
pid_file = root / 'service.pid'

def running():
    if not pid_file.exists():
        return False
    pid = int(pid_file.read_text())
    try:
        # A fixture child may await reaping by the container's init process.
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0] != 'Z'
    except FileNotFoundError:
        return False

def stop():
    if running():
        pid = int(pid_file.read_text())
        # Signal only this fixture's selected data instance, even after a
        # failed assertion or a stale marker in a subsequent fixture command.
        command = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
        assert os.fsencode(root / 'data') in command
        os.kill(pid, signal.SIGINT)
        deadline = time.monotonic() + 10
        while running():
            if time.monotonic() > deadline:
                raise RuntimeError('fixture server failed to stop')
            time.sleep(0.02)
    pid_file.unlink(missing_ok=True)

def start():
    if running():
        return
    unit = Path(os.environ['XDG_CONFIG_HOME']) / 'systemd/user' / arguments[-1]
    lines = unit.read_text().splitlines()
    command = shlex.split(next(line.removeprefix('ExecStart=') for line in lines if line.startswith('ExecStart=')))
    environment = {key: value for key, value in os.environ.items() if not key.startswith('AGENA_')}
    # A service manager uses the installed definition, not the start command's
    # ambient server options. Test passwords are local, synthetic fixtures.
    for line in lines:
        if line.startswith('Environment='):
            for assignment in shlex.split(line.removeprefix('Environment=')):
                key, value = assignment.split('=', 1)
                environment[key] = value
    with (root / 'service.log').open('ab') as output:
        process = subprocess.Popen(command, env=environment, stdin=subprocess.DEVNULL,
                                   stdout=output, stderr=output, start_new_session=True)
    pid_file.write_text(str(process.pid))

if action == 'restart':
    stop()
    start()
elif action == 'start' or (action == 'enable' and '--now' in arguments):
    start()
elif action == 'stop':
    stop()
elif action not in ('daemon-reload', 'enable', 'disable', 'reset-failed'):
    raise RuntimeError(f'unexpected fixture manager action: {action}')
'''


@unittest.skipUnless(sys.platform == 'linux' and os.environ.get('AGENA_TEST_BINARY'),
                     'Linux and AGENA_TEST_BINARY are required for isolated service lifecycle tests')
class ServerLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-service-lifecycle-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / 'workspace'
        self.workspace.mkdir()
        fake_bin = self.root / 'bin'
        fake_bin.mkdir()
        manager = fake_bin / 'systemctl'
        manager.write_text('#!' + sys.executable + '\n' + SERVICE_MANAGER)
        manager.chmod(0o755)
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith('AGENA_')}
        self.environment.update({
            'PATH': str(fake_bin) + os.pathsep + os.environ.get('PATH', '/usr/bin:/bin'),
            'XDG_CONFIG_HOME': str(self.root / 'config'),
            'AGENA_TEST_SERVICE_ROOT': str(self.root),
        })
        self.binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        self.port = self.available_port()
        self.password = 'isolated-installed-service-password'
        self.opener = build_opener(ProxyHandler({}))
        self.addCleanup(self.stop_fixture)

    @staticmethod
    def available_port():
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            return listener.getsockname()[1]

    def invoke(self, action, *options, environment=None):
        child_environment = self.environment | (environment or {})
        result = subprocess.run(
            [self.binary, '--data-dir', str(self.root / 'data'), 'server', action, *map(str, options)],
            env=child_environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=25,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        return result.stdout

    def stop_fixture(self):
        pid_file = self.root / 'service.pid'
        if pid_file.exists():
            subprocess.run([str(self.root / 'bin/systemctl'), '--user', 'stop', 'fixture.service'],
                           env=self.environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)

    def install(self, password=None):
        self.invoke('install', '--host', '127.0.0.1', '--port', self.port,
                    '--workspace', self.workspace, '--mcp-enabled', 'false',
                    '--password', password or self.password)

    def record(self):
        return json.loads((self.root / 'data/run/server.json').read_text())

    def definition(self):
        paths = list((self.root / 'config/systemd/user').glob('agena-server-*.service'))
        self.assertEqual(len(paths), 1)
        return paths[0].read_bytes()

    def manager_calls(self):
        return [json.loads(line) for line in (self.root / 'manager-calls.jsonl').read_text().splitlines()]

    def assert_password(self, password, expected):
        request = Request(f'http://127.0.0.1:{self.port}/auth/session',
                          data=json.dumps({'password': password}).encode(),
                          headers={'Content-Type': 'application/json'})
        try:
            response = self.opener.open(request, timeout=5)
        except HTTPError as error:
            response = error
        with response:
            self.assertEqual(response.status, expected)

    def test_start_is_idempotent_with_ambient_options_and_preserves_saved_configuration(self):
        self.install()
        record = self.record()
        definition = self.definition()
        calls = self.manager_calls()
        environments = [
            {},
            {'AGENA_WORKSPACE_ROOT': str(self.root / 'unselected-workspace')},
            {'AGENA_SERVER_PASSWORD': 'unselected-shell-password'},
            {'AGENA_SERVER_UI_DIR': str(self.root / 'unselected-ui')},
            {'AGENA_MCP_ENABLED': 'true'},
            {'AGENA_MCP_PUBLIC_URL': 'https://unselected.example.com/mcp'},
            {'AGENA_MCP_SERVER_URL': 'https://unselected.example.com'},
            {'AGENA_MCP_OAUTH_ISSUER_URL': 'https://unselected-auth.example.com'},
            {'AGENA_SERVER_HOST': '0.0.0.0', 'AGENA_SERVER_PORT': '1'},
        ]
        for environment in environments:
            with self.subTest(environment=list(environment)):
                output = self.invoke('start', environment=environment)
                self.assertIn('already running', output)
                self.assertIn('installed Agena user service configuration', output)
                self.assertEqual(self.record(), record)
                self.assertEqual(self.definition(), definition)
        self.assertEqual(self.manager_calls(), calls)
        self.assert_password(self.password, 200)
        self.assert_password('unselected-shell-password', 401)

    def test_start_restarts_a_stopped_service_without_rewriting_its_definition(self):
        self.install()
        record = self.record()
        definition = self.definition()
        self.invoke('stop')
        self.assertFalse((self.root / 'data/run/server.json').exists())
        output = self.invoke('start', '--workspace', self.root / 'unselected-workspace',
                             '--password', 'unselected-explicit-password',
                             '--mcp-server-url', 'https://unselected.example.com',
                             '--port', self.available_port())
        self.assertIn('Started installed Agena server', output)
        self.assertEqual(self.definition(), definition)
        self.assertNotEqual(self.record()['server_id'], record['server_id'])
        self.assertEqual(self.record()['url'], record['url'])
        self.assertEqual(self.manager_calls()[-1][0], 'start')
        self.assert_password(self.password, 200)
        self.assert_password('unselected-explicit-password', 401)

    def test_install_applies_updated_settings_to_an_already_running_service(self):
        self.install()
        previous = self.record()
        self.port = self.available_port()
        replacement = 'isolated-replacement-service-password'
        self.install(password=replacement)
        current = self.record()
        self.assertNotEqual(current['server_id'], previous['server_id'])
        self.assertEqual(current['url'], f'http://127.0.0.1:{self.port}')
        self.assertEqual(self.manager_calls()[-1][0], 'restart')
        self.assert_password(replacement, 200)
        self.assert_password(self.password, 401)

    def test_start_without_a_service_keeps_detached_launch_options(self):
        self.addCleanup(lambda: self.invoke('stop'))
        environment = {'AGENA_WORKSPACE_ROOT': str(self.workspace), 'AGENA_MCP_ENABLED': 'false'}
        output = self.invoke('start', '--host', '127.0.0.1', '--port', self.port,
                             '--password', self.password, environment=environment)
        self.assertIn('Started Agena server', output)
        record = self.record()
        self.assertEqual(record['url'], f'http://127.0.0.1:{self.port}')
        self.assert_password(self.password, 200)
        output = self.invoke('start', '--port', self.port, environment=environment)
        self.assertIn('already running', output)
        self.assertEqual(self.record(), record)
        self.assertFalse((self.root / 'manager-calls.jsonl').exists())


@unittest.skipUnless(os.environ.get('AGENA_TEST_BINARY'),
                     'AGENA_TEST_BINARY is required for session restart tests')
class SessionRestartTests(unittest.TestCase):
    """Exercise crash recovery over HTTP on every platform, without a service manager."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-session-restart-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        binary_variable = ('AGENA_TEST_PREVIOUS_BINARY'
                           if self._testMethodName == 'test_previous_schema_upgrades_with_existing_forks'
                           else 'AGENA_TEST_BINARY')
        self.binary = Path(os.environ[binary_variable]).resolve(strict=True)
        self.trace_path = self.root / 'http.jsonl'
        self.trace = http_fixture.JsonLines(self.trace_path)
        self.addCleanup(self.trace.close)
        self.provider = http_fixture.Provider(20, self.trace)
        self.addCleanup(self.provider.close)
        self.workspace = http_fixture.prepare_configuration(self.root, self.provider.base_url)
        self.environment = http_fixture.clean_environment(self.root)
        self.environment['AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES'] = '1'
        self.log = (self.root / 'server.log').open('wb')
        self.addCleanup(self.log.close)
        self.process = None
        self.api = None
        self.addCleanup(self.stop)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            self.port = listener.getsockname()[1]
        self.identity = self.start()
        self.workspace_id = self.api.call('POST', '/api/v1/workspaces/resolve', {
            'path': str(self.workspace), 'create_if_missing': True,
        })[0]['id']

    def start(self):
        self.api = http_fixture.Api(f'http://127.0.0.1:{self.port}', 20, self.trace)
        self.process = subprocess.Popen(
            [str(self.binary), '--data-dir', str(self.root / 'data'), 'server',
             '--host', '127.0.0.1', '--port', str(self.port), '--workspace', str(self.workspace)],
            cwd=self.root / 'process-cwd', env=self.environment,
            stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT,
        )
        record_path = self.root / 'data/run/server.json'

        def published():
            self.assertIsNone(self.process.poll(), 'server exited before publishing its identity')
            try:
                return json.loads(record_path.read_text())['pid'] == self.process.pid
            except (OSError, ValueError):
                return False

        http_fixture.poll(published, self.api.timeout, 'server did not replace its discovery record')
        return http_fixture.wait_server(self.process, self.api, record_path)['health']['server']

    def disconnect_client(self):
        if self.api is not None:
            connection = getattr(self.api.local, 'connection', None)
            if connection is not None:
                connection.close()
                self.api.local.connection = None

    def stop(self):
        self.disconnect_client()
        if self.process is not None:
            # kill(), rather than a graceful shutdown, must leave recovery work
            # for the next process. The child is always this fixture's Popen.
            self.process.kill()
            self.process.wait(timeout=10)
            self.process = None

    def attempts(self, session_id):
        database = (self.root / 'data/agena.db').as_uri() + '?mode=ro'
        with sqlite3.connect(database, uri=True) as connection:
            return connection.execute(
                'SELECT a.generation,a.phase FROM agena_executions e '
                'JOIN agena_attempts a ON a.execution_id=e.id '
                'WHERE e.session_id=? ORDER BY e.message_id,a.generation',
                (session_id,),
            ).fetchall()

    @unittest.skipUnless(os.environ.get('AGENA_TEST_PREVIOUS_BINARY'),
                         'requires the previous executable to create a genuine predecessor database')
    def test_previous_schema_upgrades_with_existing_forks(self):
        root = http_fixture.create_session(self.api, self.workspace_id, 'schema predecessor')
        seed = self.provider.plan('upgrade_seed', 'text', payloads=['Original completed response'])
        http_fixture.submit(self.api, root, 'UPGRADE_ORIGINAL_CONTEXT ' + 'history detail ' * 300)
        http_fixture.wait_idle(self.api, root)
        self.provider.assert_step(seed)
        branch = self.api.call('POST', f'/api/v1/sessions/{root}/fork', {})[0]['session']['id']
        before = http_fixture.read_assistant_text(self.api, branch)
        user_ids = [part['part_id'] for part in self.api.parts(branch) if part['role'] == 'user']
        database = (self.root / 'data/agena.db').as_uri() + '?mode=ro'
        with sqlite3.connect(database, uri=True) as connection:
            self.assertEqual(connection.execute(
                "SELECT count(*) FROM sqlite_schema WHERE name='agena_execution_purposes'",
            ).fetchone()[0], 0, 'the predecessor executable must create the actual old schema')
        self.stop()
        self.binary = Path(os.environ['AGENA_TEST_BINARY']).resolve(strict=True)
        upgraded = self.start()
        self.assertEqual(upgraded['instance_id'], self.identity['instance_id'])
        self.assertEqual(http_fixture.read_assistant_text(self.api, branch), before)
        self.assertEqual([part['part_id'] for part in self.api.parts(branch) if part['role'] == 'user'], user_ids)
        nested = self.api.call('POST', f'/api/v1/sessions/{branch}/fork', {})[0]['session']['id']
        summary = self.provider.plan('upgrade_compact', 'text', payloads=[
            'UPGRADED_BRANCH_SUMMARY: preserve the original branch context.',
        ])
        self.api.call('POST', f'/api/v1/sessions/{nested}/compact', {})
        http_fixture.wait_idle(self.api, nested)
        self.provider.assert_step(summary)
        response = self.provider.plan('upgrade_continue', 'text', payloads=['Upgrade continuation succeeded'])
        self.api.call('POST', f'/api/v1/sessions/{nested}/continue', {})
        http_fixture.wait_idle(self.api, nested)
        self.provider.assert_step(response)
        self.assertIn('Upgrade continuation succeeded', http_fixture.read_assistant_text(self.api, nested))
        requests = {record['step']: json.dumps(record['request'])
                    for line in self.trace_path.read_text().splitlines()
                    if (record := json.loads(line))['kind'] == 'provider_request'}
        self.assertIn('UPGRADE_ORIGINAL_CONTEXT', requests['upgrade_compact'])
        self.assertIn('UPGRADED_BRANCH_SUMMARY', requests['upgrade_continue'])
        self.assertNotIn('UPGRADE_ORIGINAL_CONTEXT', requests['upgrade_continue'])
        self.assertEqual(self.provider.errors, [])

    def test_crash_reconciles_abandoned_attempt_before_first_session_read(self):
        session = http_fixture.create_session(self.api, self.workspace_id, 'crash recovery')
        held = self.provider.plan('abandoned', 'held')
        http_fixture.submit(self.api, session, 'hold until the server crashes')
        self.assertTrue(held.requested.wait(20), 'provider did not receive the active request')
        self.assertEqual(http_fixture.state_kind(self.api.state(session)), 'running')
        self.stop()
        self.provider.assert_step(held)
        self.assertEqual(self.attempts(session), [(1, 'running')])

        restarted = self.start()
        self.assertNotEqual(restarted['id'], self.identity['id'])
        self.assertEqual(restarted['instance_id'], self.identity['instance_id'])
        # No retry/poll may hide a stale first projection of the abandoned run.
        recovered = self.api.state(session)
        self.assertEqual(http_fixture.state_kind(recovered), 'failed')
        self.assertEqual(self.attempts(session), [(1, 'interrupted')])
        user = next(part for part in self.api.parts(session)
                    if part['role'] == 'user' and part['message_boundary'])
        self.assertEqual(user['state'], 'completed')
        self.assertEqual(user['content']['execution_phase'], 'interrupted')
        self.assertEqual(user['content']['abort_reason'], 'process_restart')
        self.assertEqual(user['content']['document'], [
            {'type': 'text', 'text': 'hold until the server crashes'},
        ])

        answer = self.provider.plan('recovered', 'text', payloads=['recovered after server restart'])
        self.api.call('POST', f'/api/v1/sessions/{session}/continue', {})
        http_fixture.wait_idle(self.api, session)
        self.provider.assert_step(answer)
        self.assertIn('recovered after server restart', http_fixture.read_assistant_text(self.api, session))
        self.assertEqual(self.attempts(session), [(1, 'interrupted'), (2, 'succeeded')])
        assistant_boundaries = [part for part in self.api.parts(session)
                                if part['role'] == 'assistant' and part['message_boundary']]
        self.assertEqual(len(assistant_boundaries), 1, 'the abandoned attempt must not fabricate a reply')
        self.assertEqual(self.provider.errors, [])

    def test_fork_freezes_inherited_stream_at_its_creation_boundary(self):
        root = http_fixture.create_session(self.api, self.workspace_id, 'fork streaming boundary')
        stream = self.provider.plan('fork_stream', 'text',
                                    payloads=['BEFORE_FORK_BOUNDARY', 'AFTER_FORK_BOUNDARY'])
        http_fixture.submit(self.api, root, 'Stream an answer for the branch snapshot')
        http_fixture.poll(lambda: 'BEFORE_FORK_BOUNDARY' in
                          http_fixture.read_assistant_text(self.api, root), 20,
                          'parent did not publish its initial text')
        branch = self.api.call('POST', f'/api/v1/sessions/{root}/fork', {})[0]['session']['id']
        self.assertEqual(http_fixture.read_assistant_text(self.api, branch), 'BEFORE_FORK_BOUNDARY')
        stream.steady_gate.set()
        http_fixture.wait_idle(self.api, root)
        self.provider.assert_step(stream)
        self.assertIn('AFTER_FORK_BOUNDARY', http_fixture.read_assistant_text(self.api, root))
        self.assertEqual(http_fixture.read_assistant_text(self.api, branch), 'BEFORE_FORK_BOUNDARY')
        nested = self.api.call('POST', f'/api/v1/sessions/{branch}/fork', {})[0]['session']['id']
        self.assertEqual(http_fixture.read_assistant_text(self.api, nested), 'BEFORE_FORK_BOUNDARY')
        self.assertEqual(self.provider.errors, [])

    def test_nested_fork_can_continue_without_an_owned_user_message(self):
        root = http_fixture.create_session(self.api, self.workspace_id, 'continue inherited context')
        seed = self.provider.plan('continue_seed', 'text', payloads=['Original answer'])
        http_fixture.submit(self.api, root, 'CONTINUE_INHERITED_TASK')
        http_fixture.wait_idle(self.api, root)
        self.provider.assert_step(seed)
        nested = root
        for _ in range(2):
            nested = self.api.call('POST', f'/api/v1/sessions/{nested}/fork', {})[0]['session']['id']
        user_ids = [part['part_id'] for part in self.api.parts(nested)
                    if part['role'] == 'user' and part['message_boundary']]
        continuation = self.provider.plan('continue_nested', 'text', payloads=['CONTINUED_IN_FORK'])
        self.api.call('POST', f'/api/v1/sessions/{nested}/continue', {})
        http_fixture.wait_idle(self.api, nested)
        self.provider.assert_step(continuation)
        self.assertIn('CONTINUED_IN_FORK', http_fixture.read_assistant_text(self.api, nested))
        self.assertNotIn('CONTINUED_IN_FORK', http_fixture.read_assistant_text(self.api, root))
        self.assertEqual([part['part_id'] for part in self.api.parts(nested)
                          if part['role'] == 'user' and part['message_boundary']], user_ids)
        self.assertEqual(self.attempts(root), [(1, 'succeeded')])
        self.assertEqual(self.provider.errors, [])

    def test_paused_compaction_keeps_its_purpose_after_restart(self):
        session = http_fixture.create_session(self.api, self.workspace_id, 'queued maintenance')
        seed = self.provider.plan('compact_queue_seed', 'text', payloads=['Seed response'])
        http_fixture.submit(self.api, session, 'COMPACT_QUEUE_HISTORY ' + 'history detail ' * 200)
        http_fixture.wait_idle(self.api, session)
        self.provider.assert_step(seed)
        user_ids = [part['part_id'] for part in self.api.parts(session)
                    if part['role'] == 'user' and part['message_boundary']]
        self.api.call('POST', f'/api/v1/sessions/{session}/queue/pause', {'paused': True})
        summary = self.provider.plan('compact_queue_summary', 'text',
                                     payloads=['QUEUED_COMPACT_SUMMARY: retain the original task.'])
        self.api.call('POST', f'/api/v1/sessions/{session}/compact', {})
        self.assertTrue(self.api.state(session)['queue_paused'])
        self.assertFalse(summary.requested.is_set())
        self.stop()
        self.start()
        self.assertTrue(self.api.state(session)['queue_paused'])
        self.assertFalse(summary.requested.is_set())
        self.api.call('POST', f'/api/v1/sessions/{session}/queue/pause', {'paused': False})
        http_fixture.wait_idle(self.api, session)
        self.provider.assert_step(summary)
        parts = self.api.parts(session)
        self.assertTrue(any(part['kind'] == 'compaction' and
                            part['content'].get('outcome') == 'compacted' for part in parts))
        self.assertEqual([part['part_id'] for part in parts
                          if part['role'] == 'user' and part['message_boundary']], user_ids,
                         'maintenance must not fabricate a new user message')
        self.assertEqual(self.provider.errors, [])

    def test_other_workspace_and_nested_forks_read_their_own_files(self):
        secondary = self.root / 'secondary-workspace'
        secondary.mkdir()
        (secondary / 'permission-fixture.txt').write_text('SECONDARY_SESSION_BYTES')
        (self.workspace / 'permission-fixture.txt').write_text('WRONG_SERVER_WORKSPACE')
        workspace = self.api.call('POST', '/api/v1/workspaces/resolve', {
            'path': str(secondary), 'create_if_missing': True,
        })[0]['id']
        session = http_fixture.create_session(self.api, workspace, 'other owning workspace')
        for label in ['other_root', 'other_nested_fork']:
            if label == 'other_nested_fork':
                for _ in range(2):
                    session = self.api.call('POST', f'/api/v1/sessions/{session}/fork', {})[0]['session']['id']
            self.assertEqual(self.api.state(session)['execution']['effective_workspace_root'], str(secondary))
            http_fixture.ask_permission(self.api, session)
            tool = self.provider.plan(f'{label}_tool', 'permission')
            continuation = self.provider.plan(f'{label}_answer', 'continuation',
                                              payloads=['Read the owning workspace'])
            http_fixture.submit(self.api, session, 'Read permission-fixture.txt')
            pending = http_fixture.permission_wait(self.api, session)
            self.api.call('POST', f'/api/v1/sessions/{session}/permission-replies', {
                'reply': {'request_id': pending['request_id'], 'kind': 'allow_once'},
            })
            http_fixture.wait_idle(self.api, session)
            self.provider.assert_step(tool)
            self.provider.assert_step(continuation)
        requests = {record['step']: json.dumps(record['request'])
                    for line in self.trace_path.read_text().splitlines()
                    if (record := json.loads(line))['kind'] == 'provider_request'}
        for step in ['other_root_answer', 'other_nested_fork_answer']:
            self.assertIn('SECONDARY_SESSION_BYTES', requests[step])
            self.assertNotIn('WRONG_SERVER_WORKSPACE', requests[step])
        self.assertEqual(self.provider.errors, [])

    def test_agent_tools_wait_for_child_file_read_and_compact_without_redelivery(self):
        secondary = self.root / 'agent-workspace'
        secondary.mkdir()
        fixture_token = 'CHILD_OWNING_WORKSPACE_BYTES'
        (secondary / 'permission-fixture.txt').write_text(fixture_token)
        (self.workspace / 'permission-fixture.txt').write_text('WRONG_SERVER_WORKSPACE')
        workspace = self.api.call('POST', '/api/v1/workspaces/resolve', {
            'path': str(secondary), 'create_if_missing': True,
        })[0]['id']
        parent = http_fixture.create_session(self.api, workspace, 'real agent tools')
        database = (self.root / 'data/agena.db').as_uri() + '?mode=ro'

        def stored(query, *parameters):
            with sqlite3.connect(database, uri=True) as connection:
                return connection.execute(query, parameters).fetchall()

        def tool_phases(session):
            return stored(
                "SELECT json_extract(t.input_json,'$.tool'),t.phase FROM agena_tool_calls t "
                'JOIN agena_executions e ON e.id=t.execution_id WHERE e.session_id=? ORDER BY t.id',
                session,
            )

        def results():
            return stored(
                'SELECT id,payload_json,state,observed_round_id FROM agena_deliveries '
                "WHERE session_id=? AND json_extract(payload_json,'$.kind')='agent_result' ORDER BY id",
                parent,
            )

        # Both tool calls belong to one parent round. The actual agent.wait
        # blocks its next model request until the independently running child
        # finishes, giving the FIFO provider an unambiguous child/parent order.
        delegation = self.provider.plan('agent_spawn_wait', 'tool-calls', tool_calls=[
            {'tool': 'agent.spawn', 'input': {
                'name': 'reader',
                'prompt': 'CHILD_TASK_CONTEXT: Read permission-fixture.txt and report its bytes. '
                          + 'Preserve the delegated scope. ' * 100,
            }},
            {'tool': 'agent.wait', 'input': {'timeout_ms': 15000}},
        ])
        child_read = self.provider.plan('agent_child_read', 'permission')
        child_answer = self.provider.plan('agent_child_answer', 'text',
                                          payloads=['CHILD_WAITING_FOR_TEST\n',
                                                    'CHILD_FINAL_RESULT: ' + fixture_token],
                                          expected_tool_bytes=fixture_token)
        parent_answer = self.provider.plan('agent_parent_answer', 'text',
                                           payloads=['PARENT_RECEIVED_CHILD_RESULT'],
                                           expected_tool_bytes='CHILD_FINAL_RESULT: ' + fixture_token)
        http_fixture.submit(self.api, parent, 'PARENT_PRIVATE_CONTEXT: delegate the file read and wait.')
        self.assertTrue(child_read.requested.wait(20), 'spawn did not start the child model request')
        self.assertTrue(child_answer.requested.wait(20), 'child did not observe its successful fs.read')
        children = stored('SELECT id FROM agena_sessions WHERE parent_session_id=? AND agent_name=?',
                          parent, 'reader')
        self.assertEqual(len(children), 1, 'agent.spawn must create exactly one child')
        child = children[0][0]
        child_state = self.api.state(child)
        self.assertEqual(http_fixture.state_kind(child_state), 'running')
        self.assertEqual(child_state['session']['parent_id'], parent)
        self.assertEqual(child_state['session']['root_id'], parent)
        self.assertEqual(child_state['session']['depth'], 1)
        self.assertEqual(child_state['session']['relation_kind'], 'subagent')
        self.assertEqual(child_state['session']['agent_status'], 'running')
        self.assertEqual(child_state['execution']['effective_workspace_root'], str(secondary))
        self.assertEqual(http_fixture.pending_permissions(child_state), [])
        self.assertEqual(tool_phases(child), [('fs.read', 'succeeded')])
        parent_state = self.api.state(parent)
        self.assertEqual(http_fixture.state_kind(parent_state), 'running')
        self.assertEqual(parent_state['session']['state']['data']['workflow'], 'tool_pending')
        self.assertEqual(http_fixture.pending_permissions(parent_state), [])
        self.assertIn(('agent.wait', 'running'), tool_phases(parent))
        self.assertFalse(parent_answer.requested.is_set(), 'agent.wait must wait for the child result')
        self.assertEqual(results(), [])

        # The delegated read is already authorized by the owning workspace.
        # Subagents cannot ask users directly; hold their actual model stream
        # instead so agent.wait is observed before the child finishes.
        child_answer.steady_gate.set()
        http_fixture.wait_idle(self.api, parent)
        completed = http_fixture.wait_idle(self.api, child)
        for step in [delegation, child_read, child_answer, parent_answer]:
            self.provider.assert_step(step)
        self.assertEqual(completed['session']['agent_status'], 'completed')
        self.assertEqual(http_fixture.pending_permissions(completed), [])
        self.assertIsNone(completed['execution'].get('agent_failure'))
        self.assertIsNotNone(completed['execution'].get('agent_finished_at'))
        self.assertEqual(tool_phases(parent), [('agent.spawn', 'succeeded'), ('agent.wait', 'succeeded')])
        self.assertEqual(tool_phases(child), [('fs.read', 'succeeded')])
        wait_reason = stored(
            "SELECT json_extract(t.result_json,'$.raw_output.payload.reason') FROM agena_tool_calls t "
            'JOIN agena_executions e ON e.id=t.execution_id '
            "WHERE e.session_id=? AND json_extract(t.input_json,'$.tool')='agent.wait'", parent,
        )
        self.assertEqual(wait_reason, [('message',)], 'agent.wait must wake from delivery, not timeout')
        delivered = results()
        self.assertEqual(len(delivered), 1, 'the delegated run must deliver exactly one final result')
        result = json.loads(delivered[0][1])
        self.assertEqual(result['agent_id'], child)
        self.assertEqual(result['status'], 'completed')
        self.assertIn('CHILD_FINAL_RESULT: ' + fixture_token, result['text'])
        self.assertEqual(delivered[0][2], 'consumed')
        self.assertIsNotNone(delivered[0][3], 'the parent model must observe the delivered result')
        self.assertEqual(self.attempts(parent), [(1, 'succeeded')])
        self.assertEqual(self.attempts(child), [(1, 'succeeded')])
        self.assertIn('PARENT_RECEIVED_CHILD_RESULT', http_fixture.read_assistant_text(self.api, parent))

        parent_parts = self.api.parts(parent)
        child_users = [part['part_id'] for part in self.api.parts(child)
                       if part['role'] == 'user' and part['message_boundary']]
        summary = self.provider.plan('agent_child_compact', 'text',
                                     payloads=['COMPACTED_CHILD_CONTEXT: keep the file read result.'])
        self.api.call('POST', f'/api/v1/sessions/{child}/compact', {})
        compacted = http_fixture.wait_idle(self.api, child)
        self.provider.assert_step(summary)
        self.assertEqual(summary.facts['response_content_type'], 'application/json')
        self.assertTrue(any(part['kind'] == 'compaction' and
                            part['content'].get('outcome') == 'compacted' for part in self.api.parts(child)))
        for field in ['agent_status', 'agent_started_at', 'agent_finished_at', 'agent_failure']:
            self.assertEqual(compacted['execution'].get(field), completed['execution'].get(field), field)
        self.assertEqual(results(), delivered, 'maintenance must not redeliver the delegated answer')
        self.assertEqual(self.api.parts(parent), parent_parts)
        self.assertEqual([part['part_id'] for part in self.api.parts(child)
                          if part['role'] == 'user' and part['message_boundary']], child_users)
        self.assertEqual(self.attempts(child), [(1, 'succeeded'), (1, 'succeeded')])
        requests = {record['step']: json.dumps(record['request'])
                    for line in self.trace_path.read_text().splitlines()
                    if (record := json.loads(line))['kind'] == 'provider_request'}
        self.assertNotIn('PARENT_PRIVATE_CONTEXT', requests['agent_child_read'])
        self.assertIn(fixture_token, requests['agent_child_answer'])
        self.assertNotIn('WRONG_SERVER_WORKSPACE', requests['agent_child_answer'])
        self.assertIn('CHILD_FINAL_RESULT: ' + fixture_token, requests['agent_parent_answer'])
        self.assertIn('CHILD_TASK_CONTEXT', requests['agent_child_compact'])
        self.assertEqual(self.provider.errors, [])

    def test_nested_forks_compact_use_tools_and_resume_without_parent_state(self):
        root = http_fixture.create_session(self.api, self.workspace_id, 'independent fork root')
        seed = self.provider.plan('fork_seed', 'text', payloads=['Original response before branching'])
        http_fixture.submit(self.api, root, 'ORIGINAL_HISTORY_MARKER ' + 'history detail ' * 600)
        http_fixture.wait_idle(self.api, root)
        self.provider.assert_step(seed)
        branch = self.api.call('POST', f'/api/v1/sessions/{root}/fork', {})[0]['session']
        nested = self.api.call('POST', f'/api/v1/sessions/{branch["id"]}/fork', {})[0]['session']
        self.assertEqual(branch['parent_id'], root)
        self.assertEqual(nested['parent_id'], branch['id'])
        self.assertEqual(nested['root_id'], root)
        self.assertEqual(nested['depth'], 2)
        self.assertEqual(nested['relation_kind'], 'fork')
        branch_parts = self.api.parts(branch['id'])

        parent_run = self.provider.plan('fork_parent_running', 'held')
        http_fixture.submit(self.api, root, 'ROOT_ONLY_LIVE_MESSAGE')
        self.assertTrue(parent_run.requested.wait(20))
        summary = self.provider.plan('nested_compaction', 'text', payloads=[
            'COMPACTED_NESTED_CONTEXT: preserve the independent branch investigation.',
        ])
        self.api.call('POST', f'/api/v1/sessions/{nested["id"]}/compact', {})
        http_fixture.wait_idle(self.api, nested['id'])
        self.provider.assert_step(summary)
        self.assertTrue(any(part['kind'] == 'compaction' and
                            part['content'].get('outcome') == 'compacted'
                            for part in self.api.parts(nested['id'])))
        self.assertEqual(self.api.parts(branch['id']), branch_parts)
        self.assertEqual(http_fixture.state_kind(self.api.state(root)), 'running')

        branch_run = self.provider.plan('fork_branch_running', 'held')
        http_fixture.submit(self.api, branch['id'], 'BRANCH_ONLY_LIVE_MESSAGE')
        self.assertTrue(branch_run.requested.wait(20))
        self.api.call('POST', f'/api/v1/sessions/{root}/cancel', {})
        http_fixture.wait_idle(self.api, root)
        self.provider.assert_step(parent_run)
        self.assertEqual(http_fixture.state_kind(self.api.state(branch['id'])), 'running')
        self.assertFalse(branch_run.disconnected.is_set(), 'stopping the root must not stop a fork')
        self.api.call('POST', f'/api/v1/sessions/{branch["id"]}/cancel', {})
        http_fixture.wait_idle(self.api, branch['id'])
        self.provider.assert_step(branch_run)

        fixture_token = 'NESTED_FORK_ACTUAL_TOOL_BYTES'
        (self.workspace / 'permission-fixture.txt').write_text(fixture_token)
        http_fixture.ask_permission(self.api, nested['id'])
        tool = self.provider.plan('nested_tool', 'permission')
        continuation = self.provider.plan('nested_tool_continuation', 'continuation',
                                          payloads=['Nested tool completed'],
                                          expected_tool_bytes=fixture_token)
        http_fixture.submit(self.api, nested['id'], 'Read permission-fixture.txt')
        pending = http_fixture.permission_wait(self.api, nested['id'])
        self.assertEqual(pending['session_id'], nested['id'])
        self.assertEqual(http_fixture.pending_permissions(self.api.state(root)), [])
        self.assertEqual(http_fixture.pending_permissions(self.api.state(branch['id'])), [])
        self.api.call('POST', f'/api/v1/sessions/{nested["id"]}/permission-replies', {
            'reply': {'request_id': pending['request_id'], 'kind': 'allow_once'},
        })
        http_fixture.wait_idle(self.api, nested['id'])
        self.provider.assert_step(tool)
        self.provider.assert_step(continuation)
        self.assertIn('Nested tool completed', http_fixture.read_assistant_text(self.api, nested['id']))

        self.stop()
        self.start()
        resumed = self.provider.plan('nested_after_restart', 'text',
                                     payloads=['Nested branch resumed after restart'])
        http_fixture.submit(self.api, nested['id'], 'Continue the nested branch')
        http_fixture.wait_idle(self.api, nested['id'])
        self.provider.assert_step(resumed)
        requests = {record['step']: json.dumps(record['request'])
                    for line in self.trace_path.read_text().splitlines()
                    if (record := json.loads(line))['kind'] == 'provider_request'}
        self.assertIn('ORIGINAL_HISTORY_MARKER', requests['nested_compaction'])
        self.assertNotIn('ROOT_ONLY_LIVE_MESSAGE', requests['nested_compaction'])
        for step in ['nested_tool', 'nested_tool_continuation', 'nested_after_restart']:
            self.assertIn('COMPACTED_NESTED_CONTEXT', requests[step])
            self.assertNotIn('ORIGINAL_HISTORY_MARKER', requests[step])
            self.assertNotIn('ROOT_ONLY_LIVE_MESSAGE', requests[step])
            self.assertNotIn('BRANCH_ONLY_LIVE_MESSAGE', requests[step])
        self.assertEqual(self.provider.errors, [])

    def test_crash_restores_queued_messages_in_order_without_the_sender(self):
        session = http_fixture.create_session(self.api, self.workspace_id, 'durable queue')
        held = self.provider.plan('active_before_crash', 'held')
        http_fixture.submit(self.api, session, 'active before crash')
        self.assertTrue(held.requested.wait(20))
        http_fixture.submit(self.api, session, 'first queued prompt')
        http_fixture.submit(self.api, session, 'second queued prompt')
        self.assertEqual(len(self.api.state(session)['queued_messages']), 2)
        self.stop()
        self.provider.assert_step(held)
        first = self.provider.plan('queue_first', 'text', payloads=['first queued answer'])
        second = self.provider.plan('queue_second', 'text', payloads=['second queued answer'])

        self.start()
        # These requests must be scheduled by startup recovery without a
        # reconnecting sender, a session read, or an explicit continue command.
        self.assertTrue(first.requested.wait(20), 'first queued input did not resume on startup')
        self.assertTrue(second.requested.wait(20), 'second queued input did not resume on startup')
        self.provider.assert_step(first)
        self.provider.assert_step(second)
        completed = http_fixture.wait_idle(self.api, session)
        self.assertEqual(completed['queued_messages'], [])
        self.assertIn('second queued answer', http_fixture.read_assistant_text(self.api, session))
        self.assertEqual(self.attempts(session), [
            (1, 'interrupted'), (1, 'succeeded'), (1, 'succeeded'),
        ])
        requests = {record['step']: json.dumps(record['request'])
                    for line in self.trace_path.read_text().splitlines()
                    if (record := json.loads(line))['kind'] == 'provider_request'}
        self.assertIn('first queued prompt', requests['queue_first'])
        self.assertNotIn('second queued prompt', requests['queue_first'])
        self.assertLess(requests['queue_second'].index('first queued answer'),
                        requests['queue_second'].index('second queued prompt'))
        self.assertEqual(self.provider.errors, [])


if __name__ == '__main__':
    unittest.main()
