"""Real CLI/HTTP lifecycle tests with an isolated Linux service-manager fixture.

Run with AGENA_TEST_BINARY pointing to a freshly built Agena executable. The
fixture replaces systemctl in PATH and writes definitions only beneath a
temporary XDG_CONFIG_HOME; the machine's actual user services are never used.
"""

import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener


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


if __name__ == '__main__':
    unittest.main()
