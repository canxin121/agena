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
        self.binary = Path(os.environ['AGENA_TEST_BINARY']).resolve(strict=True)
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
