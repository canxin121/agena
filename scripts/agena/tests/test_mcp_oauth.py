"""Run with AGENA_TEST_BINARY pointing to a freshly built Agena executable.

Uses an isolated data directory and loopback listener. No external OAuth client
or browser is needed; redirect responses are inspected without following them.
"""

import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import re
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import unittest
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


@unittest.skipUnless(os.environ.get('AGENA_TEST_BINARY'), 'Set AGENA_TEST_BINARY to run HTTP OAuth integration tests')
class McpOAuthTests(unittest.TestCase):
    DEFAULT_PASSWORD = 'isolated-shared-server-password'

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='agena-mcp-oauth-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / 'workspace'
        self.workspace.mkdir()
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            self.port = listener.getsockname()[1]
        self.base = f'http://127.0.0.1:{self.port}'
        self.opener = build_opener(ProxyHandler({}), NoRedirect())
        self.process = None
        self.addCleanup(self.stop)
        self.start(initial=True)

    def start(self, initial=False, password=DEFAULT_PASSWORD, extra_environment=None):
        environment = {key: value for key, value in os.environ.items() if not key.startswith('AGENA_MCP_')}
        environment.pop('AGENA_SERVER_PASSWORD', None)
        environment.pop('AGENA_SERVER_UI_PASSWORD', None)
        self.password = password
        self.management_headers = {}
        args = [str(Path(os.environ['AGENA_TEST_BINARY']).resolve()), '--data-dir', str(self.root / 'data'),
                'server', '--host', '127.0.0.1', '--port', str(self.port), '--workspace', str(self.workspace)]
        if initial:
            args += ['--mcp-enabled', 'false', '--mcp-server-url', f'127.0.0.1:{self.port}']
        if password is not None:
            environment['AGENA_SERVER_PASSWORD'] = password
        for key, value in (extra_environment or {}).items():
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value
        self.process = subprocess.Popen(args, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                self.fail(f'Isolated Agena server exited with status {self.process.returncode}')
            try:
                status, _, _ = self.request('/api/v1/server/mcp')
                if status in (200, 401):
                    if password is not None:
                        session = self.json_request('/auth/session', 'POST', {'password': password})
                        self.management_headers = {'Authorization': 'Bearer ' + session['token']}
                    return
            except (OSError, URLError):
                pass
            time.sleep(0.05)
        self.fail('Isolated Agena server did not become ready within 30 seconds')

    def stop(self):
        if self.process is None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.process = None

    def request(self, path, method='GET', payload=None, form=False, headers=None):
        request_headers = dict(headers or {})
        data = None
        if payload is not None:
            data = (urlencode(payload) if form else json.dumps(payload)).encode()
            request_headers['Content-Type'] = 'application/x-www-form-urlencoded' if form else 'application/json'
        request = Request(self.base + path, data=data, headers=request_headers, method=method)
        try:
            response = self.opener.open(request, timeout=5)
        except HTTPError as error:
            response = error
        with response:
            body = response.read()
            if 'application/json' in response.headers.get('Content-Type', ''):
                body = json.loads(body)
            return response.code, response.headers, body

    def json_request(self, path, method='GET', payload=None, expected=200, **kwargs):
        if path.startswith('/api/v1/server/mcp') and 'headers' not in kwargs:
            kwargs['headers'] = self.management_headers
        status, _, body = self.request(path, method, payload, **kwargs)
        self.assertEqual(status, expected, f'{method} {path}: {body!r}')
        return body

    def authorization_request(self):
        client = self.json_request('/register', 'POST', {
            'client_name': 'Isolated OAuth integration test',
            'redirect_uris': ['https://client.example.com/callback'],
            'token_endpoint_auth_method': 'none',
        }, expected=201)
        verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk'
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
        authorization = {
            'response_type': 'code', 'client_id': client['client_id'],
            'redirect_uri': 'https://client.example.com/callback', 'state': 'integration-state',
            'code_challenge': challenge, 'code_challenge_method': 'S256',
        }
        return verifier, authorization

    def authorize(self, password):
        verifier, authorization = self.authorization_request()
        status, headers, _ = self.request('/oauth/authorize?' + urlencode(authorization))
        self.assertEqual(status, 200)
        self.assertIn("frame-ancestors 'none'", headers['Content-Security-Policy'])
        status, _, _ = self.request('/oauth/authorize', 'POST', authorization | {'password': 'incorrect'}, form=True)
        self.assertEqual(status, 401)
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {'password': password}, form=True)
        self.assertEqual(status, 303)
        callback = urlsplit(headers['Location'])
        self.assertEqual(callback.hostname, 'client.example.com')
        parameters = parse_qs(callback.query)
        self.assertEqual(parameters['iss'], [self.base])
        self.assertEqual(parameters['state'], ['integration-state'])
        exchange = {
            'grant_type': 'authorization_code', 'client_id': authorization['client_id'],
            'redirect_uri': authorization['redirect_uri'], 'code': parameters['code'][0],
            'code_verifier': verifier,
        }
        tokens = self.json_request('/oauth/token', 'POST', exchange, form=True)
        self.json_request('/oauth/token', 'POST', exchange, form=True, expected=400)
        return authorization['client_id'], tokens

    def assert_token_accepted(self, token, expected):
        # Unknown tool names exercise authentication and routing without
        # executing a filesystem, shell, network or other operational tool.
        status, _, body = self.request('/mcp', 'POST', {
            'jsonrpc': '2.0', 'id': 1,
            'method': 'tools/call', 'params': {
                'name': '__oauth_integration_probe__', 'arguments': {},
                '_meta': {'io.modelcontextprotocol/protocolVersion': '2025-11-25'},
            },
        }, headers={
            'Authorization': f'Bearer {token}', 'Accept': 'application/json, text/event-stream',
            'MCP-Protocol-Version': '2025-11-25',
        })
        if expected:
            self.assertEqual(status, 404)
            self.assertIn('__oauth_integration_probe__', body.get('error', {}).get('message', ''))
        else:
            self.assertEqual(status, 200)
        challenged = 'mcp/www_authenticate' in body.get('result', {}).get('_meta', {})
        self.assertEqual(challenged, not expected)

    def test_authorization_rotation_revocation_and_single_address_control(self):
        control_path = '/api/v1/server/mcp'
        initial = self.json_request(control_path)
        self.assertFalse(initial['enabled'])
        self.assertEqual(initial['serverUrl'], self.base)
        self.json_request('/.well-known/oauth-authorization-server', expected=404)
        enabled = self.json_request(control_path, 'PUT', {'enabled': True})
        self.assertTrue(enabled['ready'])
        password = self.password
        self.assertTrue(enabled['oauth']['passwordConfigured'])
        self.assertEqual(enabled['oauth']['passwordSource'], 'server')
        self.assertNotIn('generatedOauthPassword', enabled)
        self.assertNotIn('fallbackToUiPassword', enabled['oauth'])
        for method in ('PUT', 'DELETE'):
            self.json_request(control_path + '/oauth/password', method, expected=404)
        metadata = self.json_request('/.well-known/oauth-authorization-server')
        self.assertEqual(metadata['issuer'], self.base)
        self.assertEqual(metadata['token_endpoint'], self.base + '/oauth/token')
        self.assertEqual(metadata['token_endpoint_auth_methods_supported'], ['none'])
        resource = self.json_request('/.well-known/oauth-protected-resource/mcp')
        self.assertEqual(resource['resource'], self.base + '/mcp')
        self.assertEqual(resource['authorization_servers'], [self.base])
        catalog = self.json_request('/mcp', 'POST', {
            'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list',
            'params': {'_meta': {'io.modelcontextprotocol/protocolVersion': '2025-11-25'}},
        }, headers={'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-11-25'})
        for tool in catalog['result']['tools']:
            self.assertIn(tool['securitySchemes'][0]['type'], ('oauth2', 'noauth'))
        forwarded = self.json_request('/.well-known/oauth-authorization-server', headers={
            'X-Forwarded-Host': 'attacker.example.com', 'X-Forwarded-Proto': 'https',
        })
        self.assertEqual(forwarded['issuer'], self.base)
        self.json_request('/.well-known/oauth-authorization-server', headers={'Host': 'attacker.example.com'}, expected=400)
        self.json_request('/.well-known/oauth-authorization-server', headers={'Origin': 'https://attacker.example.com'}, expected=403)
        self.assert_token_accepted('invalid', False)
        client, issued = self.authorize(password)
        self.assert_token_accepted(issued['access_token'], True)
        rotated = self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': issued['refresh_token'],
        }, form=True)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': issued['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': rotated['refresh_token'],
        }, form=True, expected=400)
        client, revocable = self.authorize(password)
        self.json_request('/oauth/revoke', 'POST', {'token': revocable['refresh_token'], 'client_id': client}, form=True)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client, 'refresh_token': revocable['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/oauth/revoke', 'POST', {'token': revocable['access_token'], 'client_id': client}, form=True)
        self.assert_token_accepted(revocable['access_token'], False)
        old_client, before_password_change = self.authorize(password)
        self.stop()
        self.start(password=password)
        self.assert_token_accepted(before_password_change['access_token'], True)
        after_restart = self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': old_client,
            'refresh_token': before_password_change['refresh_token'],
        }, form=True)
        password = 'isolated-integration-password-replacement'
        self.stop()
        self.start(password=password)
        self.assert_token_accepted(before_password_change['access_token'], False)
        self.assert_token_accepted(after_restart['access_token'], False)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': old_client,
            'refresh_token': after_restart['refresh_token'],
        }, form=True, expected=400)
        self.json_request('/auth/session', 'POST', {'password': self.DEFAULT_PASSWORD}, expected=401)
        _, old_authorization = self.authorization_request()
        status, _, _ = self.request('/oauth/authorize', 'POST', old_authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 401)
        _, before_disable = self.authorize(password)
        self.json_request(control_path, 'PUT', {'enabled': False})
        self.json_request('/.well-known/oauth-authorization-server', expected=404)
        self.json_request(control_path, 'PUT', {'enabled': True})
        self.assert_token_accepted(before_disable['access_token'], False)
        self.json_request(control_path, 'PUT', {
            'publicUrl': 'https://tunnel.example.com/v1/mcp/existing-tunnel',
            'oauthIssuerUrl': 'https://auth.example.com',
        })
        unified = self.json_request(control_path, 'PUT', {'serverUrl': 'agena.example.com:8443'})
        self.assertEqual(unified['serverUrl'], 'https://agena.example.com:8443')
        self.assertEqual(unified['resourceUrl'], 'https://agena.example.com:8443/mcp')
        self.assertEqual(unified['oauth']['issuer'], 'https://agena.example.com:8443')
        self.assertIsNone(unified['oauthIssuerUrl'])
        self.assertTrue(unified['ready'])
        self.assertFalse(unified['warnings'])
        self.json_request(control_path, 'PUT', {'serverUrl': 'http://192.168.1.2:3210'}, expected=400)
        cleared = self.json_request(control_path, 'PUT', {'serverUrl': None})
        self.assertIsNone(cleared['serverUrl'])
        self.assertEqual(cleared['resourceUrl'], self.base + '/mcp')
        self.assertEqual(cleared['oauth']['issuer'], self.base)

    def test_one_server_password_protects_management_and_oauth(self):
        password = self.password
        control_path = '/api/v1/server/mcp'
        self.json_request(control_path, expected=401, headers={})
        self.json_request('/auth/session', 'POST', {'password': 'incorrect'}, expected=401)
        enabled = self.json_request(control_path, 'PUT', {'enabled': True})
        self.assertTrue(enabled['ready'])
        self.assertEqual(enabled['oauth']['passwordSource'], 'server')
        self.assertNotIn('generatedOauthPassword', enabled)
        _, tokens = self.authorize(password)
        self.assert_token_accepted(tokens['access_token'], True)
        self.json_request(control_path, headers={'Authorization': 'Bearer ' + tokens['access_token']}, expected=401)
        remote = self.json_request(control_path, 'PUT', {
            'serverUrl': 'https://agena.example.com',
        })
        self.assertTrue(remote['ready'])
        self.assertFalse(remote['warnings'])

    def tui_environment(self):
        return {
            **{key: value for key, value in os.environ.items() if not key.startswith('AGENA_')},
            'AGENA_DATA_DIR': str(self.root / 'data'),
            'AGENA_SERVER_URL': self.base,
            'TERM': 'xterm-256color',
        }

    def test_tui_uses_server_password_environment_for_default_and_explicit_launch(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for command in ([], ['tui']):
            with self.subTest(command=command):
                result = subprocess.run(
                    [binary, *command], cwd=self.workspace,
                    env=self.tui_environment() | {'AGENA_SERVER_PASSWORD': self.password},
                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                )
                # The authenticated handshake, runtime reads and workspace
                # resolution must all finish before terminal setup rejects pipes.
                self.assertIn('requires both stdin and stdout', result.stderr)
                self.assertNotIn('cannot connect TUI', result.stderr)
                self.assertNotIn(self.password, result.stdout + result.stderr)

        # A browser login token remains a compatible optional client credential.
        token = self.management_headers['Authorization'].removeprefix('Bearer ')
        result = subprocess.run(
            [binary, 'tui'], cwd=self.workspace,
            env=self.tui_environment() | {'AGENA_SERVER_TOKEN': token},
            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
        )
        self.assertIn('requires both stdin and stdout', result.stderr)
        self.assertNotIn(token, result.stdout + result.stderr)

    def write_client_environment(self, data_dir=None, password=None):
        installation = self.root / 'application files'
        installation.mkdir(exist_ok=True)
        client_env = installation / 'client-env.sh'
        result = subprocess.run(
            ['bash', '--noprofile', '--norc', '-c',
             'printf "export AGENA_DATA_DIR=%q\\nexport AGENA_SERVER_PASSWORD=%q\\n" "$1" "$2"',
             'client-env-fixture', str(data_dir or self.root / 'data'),
             self.password if password is None else password],
            capture_output=True, check=True,
        )
        client_env.write_bytes(result.stdout)
        client_env.chmod(0o600)
        return client_env, self.tui_environment() | {'AGENA_INSTALL_DIR': str(installation)}

    def test_tui_uses_saved_local_password_without_shell_environment(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        # Exercise both printf %q forms, including literals that would be
        # dangerous to evaluate as shell code and ANSI-C quoted control bytes.
        marker = self.root / 'must-not-execute'
        passwords = [self.password, f'local $pass `touch {marker}` quotes\'" \\ 密码',
                     'local\tpassword\nwith\x1bcontrol']
        for password in passwords:
            if password != self.password:
                self.stop()
                self.start(password=password)
            client_env, environment = self.write_client_environment()
            with client_env.open('a') as stream:
                stream.write(f'\ntouch "{marker}"\n')
            for command in ([], ['tui']):
                with self.subTest(command=command, ansi_quoted="$'" in client_env.read_text()):
                    result = subprocess.run(
                        [binary, *command], cwd=self.workspace, env=environment,
                        stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                    )
                    self.assertIn('requires both stdin and stdout', result.stderr)
                    self.assertNotIn('requires a password', result.stderr)
                    self.assertNotIn('Server password:', result.stdout + result.stderr)
                    self.assertNotIn(password, result.stdout + result.stderr)
                    self.assertFalse(marker.exists(), 'client environment was executed as shell code')

    def test_tui_saved_password_is_scoped_to_private_matching_local_instance(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        client_env, environment = self.write_client_environment()
        record_path = self.root / 'data/run/server.json'
        original_record = record_path.read_bytes()
        original_env = client_env.read_bytes()
        for invalid in ('different-data-dir', 'different-server-id', 'different-port',
                        'public-file', 'symlink-file', 'explicit-password'):
            with self.subTest(invalid=invalid):
                record_path.write_bytes(original_record)
                client_env.unlink()
                client_env.write_bytes(original_env)
                client_env.chmod(0o600)
                selected_environment = dict(environment)
                if invalid == 'different-data-dir':
                    self.write_client_environment(data_dir=self.root / 'other-instance')
                elif invalid in ('different-server-id', 'different-port'):
                    record = json.loads(original_record)
                    if invalid == 'different-server-id':
                        record['server_id'] = '00000000-0000-0000-0000-000000000000'
                    else:
                        record['url'] = 'http://127.0.0.1:1'
                    record_path.write_text(json.dumps(record))
                elif invalid == 'public-file':
                    client_env.chmod(0o644)
                elif invalid == 'symlink-file':
                    target = client_env.with_suffix('.target')
                    target.write_bytes(original_env)
                    target.chmod(0o600)
                    client_env.unlink()
                    client_env.symlink_to(target)
                else:
                    selected_environment['AGENA_SERVER_PASSWORD'] = 'explicit-incorrect-password'
                result = subprocess.run(
                    [binary, 'tui'], cwd=self.workspace, env=selected_environment,
                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                )
                expected = 'password is incorrect' if invalid == 'explicit-password' else 'requires a password'
                self.assertIn(expected, result.stderr)
                self.assertNotIn(self.password, result.stdout + result.stderr)
        record_path.write_bytes(original_record)

    @unittest.skipUnless(os.name == 'posix', 'Terminal restoration requires POSIX PTYs')
    def test_tui_hub_ctrl_c_restores_terminal_with_missing_protocol_replies(self):
        import fcntl
        import importlib.util
        from http.client import HTTPException
        import pty
        import select
        import struct
        import termios

        spec = importlib.util.spec_from_file_location(
            'tui_screen_fixture', Path(__file__).resolve().parents[2] / 'probes/tui_experience.py')
        tui_screen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tui_screen)
        tui_screen.ROWS, tui_screen.COLS = 35, 120

        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        _, environment = self.write_client_environment()
        # This is an isolated tmux helper, never the user's running tmux server.
        helpers = self.root / 'helpers'
        helpers.mkdir()
        tmux = helpers / 'tmux'
        tmux.write_text('#!/bin/sh\ncase "$1" in\nshow-options) echo off;;\n'
                        'display-message) echo xterm-256color;;\nesac\n')
        tmux.chmod(0o755)
        for mode in ('responsive', 'catalog-loading', 'no-startup-barrier',
                     'no-refresh-barrier', 'tmux-no-passthrough'):
            for command in ([], ['tui']):
                with self.subTest(mode=mode, command=command,
                                  keyboard='legacy' if not command else 'CSI-u'):
                    selected_environment = dict(environment)
                    for key in ('TMUX', 'TMUX_PANE', 'STY', 'ZELLIJ', 'SSH_CONNECTION', 'SSH_TTY'):
                        selected_environment.pop(key, None)
                    selected_environment['TERM_PROGRAM'] = 'xterm'
                    if mode == 'tmux-no-passthrough':
                        selected_environment.update(
                            TMUX='/isolated-fixture,1,0', TERM='tmux-256color',
                            PATH=str(helpers) + os.pathsep + os.environ.get('PATH', ''),
                        )
                    proxy = None
                    catalog_pending, release_catalog = threading.Event(), threading.Event()
                    if mode == 'catalog-loading':
                        base = self.base

                        class PendingHubProxy(BaseHTTPRequestHandler):
                            pending = catalog_pending
                            release = release_catalog

                            def forward(self):
                                if self.command == 'GET' and self.path.split('?')[0] == '/api/v1/sessions':
                                    self.pending.set()
                                    self.release.wait(timeout=20)
                                length = int(self.headers.get('Content-Length', 0))
                                request = Request(base + self.path,
                                                  data=self.rfile.read(length) if length else None,
                                                  headers=dict(self.headers), method=self.command)
                                try:
                                    response = build_opener(ProxyHandler({})).open(request, timeout=5)
                                except HTTPError as error:
                                    response = error
                                with response:
                                    content_type = response.headers.get('Content-Type', '')
                                    if content_type.startswith('text/event-stream'):
                                        # Live subscriptions must stream through
                                        # the proxy while the catalog is held.
                                        self.send_response(response.status)
                                        self.send_header('Content-Type', content_type)
                                        self.end_headers()
                                        try:
                                            while block := response.read1(65536):
                                                self.wfile.write(block)
                                                self.wfile.flush()
                                        except (OSError, HTTPException):
                                            pass  # Either endpoint may close during TUI shutdown.
                                        return
                                    body = response.read()
                                    self.send_response(response.status)
                                    self.send_header('Content-Type', content_type)
                                    self.send_header('Content-Length', str(len(body)))
                                    self.end_headers()
                                    try:
                                        self.wfile.write(body)
                                    except (BrokenPipeError, ConnectionResetError):
                                        pass

                            do_GET = do_POST = do_PUT = forward

                            def log_message(self, *_args):
                                pass

                        proxy = ThreadingHTTPServer(('127.0.0.1', 0), PendingHubProxy)
                        threading.Thread(target=proxy.serve_forever, daemon=True).start()
                        selected_environment.update(
                            AGENA_SERVER_URL=f'http://127.0.0.1:{proxy.server_port}',
                            AGENA_SERVER_PASSWORD=self.password,
                        )
                    master, slave = pty.openpty()
                    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 35, 120, 0, 0))
                    before = termios.tcgetattr(slave)
                    process = subprocess.Popen(
                        [binary, *command], cwd=self.workspace, env=selected_environment,
                        stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
                    )
                    try:
                        output = bytearray()
                        screen = tui_screen.Screen()
                        queries = {
                            b'\x1b[6n': b'\x1b[1;1R', b'\x1b[5n': b'\x1b[0n',
                            b'\x1b[?u': b'\x1b[?0u', b'\x1b[c': b'\x1b[?1;2c',
                            b'\x1b[0c': b'\x1b[?1;2c',
                            b'\x1b]11;?\x07': b'\x1b]11;rgb:1010/1010/1010\x1b\\',
                            b'\x1b]4;-2;?\x07': b'\x1b]4;-2;rgb:1010/1010/1010\x1b\\',
                        }
                        answered = {query: 0 for query in queries}
                        cursor_queries = 0
                        hub_visible = focus_sent = False
                        focus_query_count = 0
                        interrupt_at = None
                        deadline = time.monotonic() + 20
                        while time.monotonic() < deadline:
                            if select.select([master], [], [], 0.03)[0]:
                                block = os.read(master, 65536)
                                output.extend(block)
                                screen.feed(block)
                            pending_replies = []
                            for query, reply in queries.items():
                                matches = list(re.finditer(re.escape(query), output))
                                pending_replies.extend((match.start(), query, reply)
                                                       for match in matches[answered[query]:])
                                answered[query] = len(matches)
                            for _, query, reply in sorted(pending_replies):
                                if query == b'\x1b[6n':
                                    cursor_queries += 1
                                # Ratatui's synchronous setup needs its first
                                # CPR; withhold the runtime's final barrier.
                                if (query == b'\x1b[6n' and cursor_queries > 1 and mode == 'no-startup-barrier'
                                        or query == b'\x1b[5n' and mode == 'no-refresh-barrier' and focus_sent):
                                    continue
                                os.write(master, reply)
                            visible = screen.text()
                            if 'Session Hub' in visible:
                                hub_visible = True
                            ready_to_quit = hub_visible and 'Loading...' not in visible
                            if mode == 'catalog-loading':
                                ready_to_quit = hub_visible and catalog_pending.is_set() and 'Loading...' in visible
                            if mode == 'no-refresh-barrier' and hub_visible:
                                if not focus_sent:
                                    focus_query_count = output.count(b'\x1b[5n')
                                    os.write(master, b'\x1b[I')
                                    focus_sent = True
                                ready_to_quit = output.count(b'\x1b[5n') > focus_query_count
                            if ready_to_quit and interrupt_at is None:
                                os.write(master, b'\x03' if not command else b'\x1b[99;5u')
                                interrupt_at = time.monotonic()
                                deadline = min(deadline, interrupt_at + 3)
                            if process.poll() is not None:
                                while select.select([master], [], [], 0)[0]:
                                    output.extend(os.read(master, 65536))
                                break
                        diagnostic = (output[:6000] + output[-500:]).decode(errors='replace')
                        self.assertTrue(hub_visible, diagnostic)
                        self.assertIsNotNone(interrupt_at, diagnostic)
                        self.assertEqual(process.poll(), 0, diagnostic)
                        self.assertLess(time.monotonic() - interrupt_at, 3)
                        self.assertNotIn(b'Ctrl+C again', output)
                        self.assertNotIn(b'Server password:', output)
                        self.assertNotIn(b'failed to restore the terminal', output)
                        self.assertNotIn(b'protocol barrier within', output)
                        self.assertIn(b'\x1b[?1049l', output)
                        self.assertIn(b'\x1b[?25h', output)
                        self.assertEqual(termios.tcgetattr(slave), before)
                        if mode == 'no-startup-barrier':
                            self.assertGreaterEqual(cursor_queries, 2)
                        if mode == 'tmux-no-passthrough':
                            self.assertNotIn(b'\x1bPtmux;', output)
                    finally:
                        release_catalog.set()
                        if process.poll() is None:
                            process.kill()
                        process.wait(timeout=5)
                        os.close(master)
                        os.close(slave)
                        if proxy is not None:
                            proxy.shutdown()
                            proxy.server_close()

    def test_tui_reports_missing_or_incorrect_server_password_as_configuration(self):
        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for password, expected in ((None, 'requires a password'), ('incorrect-do-not-echo', 'password is incorrect')):
            with self.subTest(password_configured=password is not None):
                environment = self.tui_environment()
                if password is not None:
                    environment['AGENA_SERVER_PASSWORD'] = password
                result = subprocess.run(
                    [binary], cwd=self.workspace, env=environment,
                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('configuration error:', result.stderr)
                self.assertIn(expected, result.stderr)
                self.assertIn('AGENA_SERVER_PASSWORD', result.stderr)
                self.assertNotIn('UI authentication', result.stderr)
                if password is not None:
                    self.assertNotIn(password, result.stdout + result.stderr)

    @unittest.skipUnless(os.name == 'posix', 'PTY password input requires POSIX')
    def test_tui_password_prompt_hides_input_and_restores_terminal_on_failure_or_cancel(self):
        import pty
        import select
        import termios

        binary = str(Path(os.environ['AGENA_TEST_BINARY']).resolve())
        for entered, expected in (
            (b'wrong-hidden-password\r', 'password is incorrect'),
            (b'cancel-hidden-password\x03', 'password entry cancelled'),
        ):
            with self.subTest(expected=expected):
                master, slave = pty.openpty()
                before = termios.tcgetattr(slave)
                process = subprocess.Popen(
                    [binary, 'tui'], cwd=self.workspace, env=self.tui_environment(),
                    stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
                )
                try:
                    output = bytearray()
                    sent = False
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            output.extend(os.read(master, 65536))
                        if not sent and b'Server password: ' in output:
                            os.write(master, entered)
                            sent = True
                        if process.poll() is not None:
                            while select.select([master], [], [], 0)[0]:
                                output.extend(os.read(master, 65536))
                            break
                    self.assertIsNotNone(process.poll(), 'password prompt did not finish')
                    self.assertTrue(sent, output.decode(errors='replace'))
                    self.assertIn(expected.encode(), output)
                    self.assertNotIn(entered.split(b'\r')[0].split(b'\x03')[0], output)
                    self.assertEqual(termios.tcgetattr(slave), before)
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)
                    os.close(master)
                    os.close(slave)

    def test_client_metadata_uses_configured_proxy_dns_and_still_rejects_local_destinations(self):
        connections = []

        class ProxyFixture(BaseHTTPRequestHandler):
            def do_CONNECT(self):
                connections.append(self.path)
                self.send_response(502)
                self.send_header('Content-Length', '0')
                self.end_headers()

            def log_message(self, *args):
                pass

        proxy = ThreadingHTTPServer(('127.0.0.1', 0), ProxyFixture)
        worker = threading.Thread(target=proxy.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(proxy.server_close)
        self.addCleanup(proxy.shutdown)
        self.stop()
        environment = {name: None for name in (
            'http_proxy', 'HTTP_PROXY', 'https_proxy', 'HTTPS_PROXY',
            'all_proxy', 'ALL_PROXY', 'no_proxy', 'NO_PROXY',
        )}
        environment['https_proxy'] = f'http://127.0.0.1:{proxy.server_port}'
        self.start(extra_environment=environment)
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, authorization = self.authorization_request()
        # This hostname cannot resolve locally. CONNECT must reach the selected
        # proxy, whose deliberate failure still rejects the unvalidated client.
        status, headers, body = self.request('/oauth/authorize?' + urlencode(authorization | {
            'client_id': 'https://metadata-cannot-resolve.invalid/client.json',
        }))
        self.assertEqual(status, 400)
        self.assertEqual(body['error'], 'invalid_client')
        self.assertNotIn('Location', headers)
        # Provider version discovery can also use this process-wide proxy at
        # startup. Check the metadata destination independently of that traffic.
        self.assertEqual(connections.count('metadata-cannot-resolve.invalid:443'), 1)
        for client_id in (
            'https://127.0.0.1/client.json', 'https://[::1]/client.json',
            'https://198.18.1.80/client.json', 'https://169.254.169.254/client.json',
            'https://10.0.0.5/client.json', 'https://service.local/client.json',
        ):
            status, _, body = self.request('/oauth/authorize?' + urlencode(authorization | {
                'client_id': client_id,
            }))
            self.assertEqual(status, 400)
            self.assertEqual(body['error'], 'invalid_client')
        self.assertEqual(connections.count('metadata-cannot-resolve.invalid:443'), 1)
        self.assertFalse(set(connections).intersection({
            '127.0.0.1:443', '[::1]:443', '198.18.1.80:443',
            '169.254.169.254:443', '10.0.0.5:443', 'service.local:443',
        }))

    def test_login_and_oauth_share_failed_password_limit(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, authorization = self.authorization_request()
        for _ in range(7):
            self.json_request('/auth/session', 'POST', {'password': 'incorrect'}, expected=401)
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'password': 'incorrect',
        }, form=True)
        self.assertEqual(status, 429)
        self.assertGreater(int(headers['Retry-After']), 0)
        self.json_request('/auth/session', 'POST', {'password': self.password}, expected=429)

    def test_retired_password_environment_cannot_silently_disable_login(self):
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith('AGENA_MCP_')}
        environment.pop('AGENA_SERVER_PASSWORD', None)
        environment['AGENA_SERVER_UI_PASSWORD'] = 'legacy-password-must-not-be-logged'
        result = subprocess.run([
            str(Path(os.environ['AGENA_TEST_BINARY']).resolve()),
            '--data-dir', str(self.root / 'legacy-launch'), 'server',
            '--host', '127.0.0.1', '--port', '0', '--workspace', str(self.workspace),
        ], env=environment, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        diagnostics = result.stdout + result.stderr
        self.assertIn('AGENA_SERVER_UI_PASSWORD has been renamed to AGENA_SERVER_PASSWORD', diagnostics)
        self.assertNotIn(environment['AGENA_SERVER_UI_PASSWORD'], diagnostics)

    def test_removing_server_password_revokes_grants_and_never_generates_an_mcp_password(self):
        control_path = '/api/v1/server/mcp'
        self.json_request(control_path, 'PUT', {'enabled': True})
        client, tokens = self.authorize(self.password)
        self.stop()
        self.start(password=None)
        session = self.json_request('/auth/session')
        self.assertTrue(session['disabled'])
        control = self.json_request(control_path)
        self.assertFalse(control['ready'])
        self.assertFalse(control['oauth']['passwordConfigured'])
        self.assertNotIn('generatedOauthPassword', control)
        self.assertTrue(any('AGENA_SERVER_PASSWORD' in warning for warning in control['warnings']))
        self.assert_token_accepted(tokens['access_token'], False)
        self.json_request('/oauth/token', 'POST', {
            'grant_type': 'refresh_token', 'client_id': client,
            'refresh_token': tokens['refresh_token'],
        }, form=True, expected=400)
        # Password removal also clears registrations; a newly registered client
        # receives an explicit unavailable response rather than a new password.
        _, authorization = self.authorization_request()
        status, _, body = self.request('/oauth/authorize', 'POST', authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 503)
        self.assertIn(b'password', body.lower())
        self.stop()
        self.start()
        self.assert_token_accepted(tokens['access_token'], False)

    def test_legacy_mcp_password_is_discarded_and_its_grants_are_revoked(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        _, tokens = self.authorize(self.password)
        self.stop()
        with sqlite3.connect(self.root / 'data/agena.db') as database:
            control = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='mcp.server.control'",
            ).fetchone()[0])
            old_verifier = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='auth.serverPasswordVerifier'",
            ).fetchone()[0])
            control['oauth_password_phc'] = old_verifier
            control.pop('server_password_fingerprint', None)
            database.execute("DELETE FROM agena_server_state WHERE key='auth.serverPasswordVerifier'")
            database.execute("UPDATE agena_server_state SET value_json=? WHERE key='mcp.server.control'",
                             (json.dumps(control),))
        self.start(password='shared-password-after-migration')
        self.assert_token_accepted(tokens['access_token'], False)
        _, authorization = self.authorization_request()
        status, _, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'password': self.DEFAULT_PASSWORD,
        }, form=True)
        self.assertEqual(status, 401)
        _, migrated = self.authorize(self.password)
        self.assert_token_accepted(migrated['access_token'], True)
        with sqlite3.connect(self.root / 'data/agena.db') as database:
            saved = json.loads(database.execute(
                "SELECT value_json FROM agena_server_state WHERE key='mcp.server.control'",
            ).fetchone()[0])
        self.assertNotIn('oauth_password_phc', saved)
        self.assertTrue(saved['server_password_fingerprint'])

    def test_browser_consent_errors_and_passwordless_cancellation(self):
        self.json_request('/api/v1/server/mcp', 'PUT', {'enabled': True})
        password = self.password
        client = self.json_request('/register', 'POST', {
            'client_name': '<script>alert(1)</script> {{SCRIPT}} & "client"',
            'redirect_uris': ['https://client.example.com/callback'],
            'token_endpoint_auth_method': 'none',
        }, expected=201)
        verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk'
        authorization = {
            'response_type': 'code', 'client_id': client['client_id'],
            'redirect_uri': 'https://client.example.com/callback',
            'state': 'browser-state<&"', 'code_challenge_method': 'S256',
            'code_challenge': base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('='),
            'scope': 'agena:tools',
        }
        browser = {'Accept': 'text/html', 'Accept-Language': 'en-US,en;q=0.9'}
        status, headers, body = self.request('/oauth/authorize?' + urlencode(authorization), headers=browser)
        self.assertEqual(status, 200)
        html = body.decode()
        self.assertIn('lang="en-US"', html)
        self.assertIn('Connect to Agena', html)
        self.assertIn('&lt;script&gt;alert(1)&lt;/script&gt; {{SCRIPT}} &amp; &quot;client&quot;', html)
        self.assertNotIn('<script>alert(1)', html)
        self.assertNotIn('data-i18n="offline_title"', html)
        self.assertEqual(html.count('name="client_id"'), 1)
        inline_script = re.search(r'<script>(.*?)</script>', html, re.S).group(1)
        script_hash = base64.b64encode(hashlib.sha256(inline_script.encode()).digest()).decode()
        policy = headers['Content-Security-Policy']
        self.assertIn("script-src 'sha256-" + script_hash + "'", policy)
        self.assertIn("form-action 'self' https://client.example.com", policy)
        self.assertIn("frame-ancestors 'none'", policy)
        self.assertEqual(headers['X-Content-Type-Options'], 'nosniff')
        self.assertEqual(headers['X-Frame-Options'], 'DENY')
        self.assertEqual(headers['Referrer-Policy'], 'no-referrer')
        self.assertEqual(headers['Cache-Control'], 'no-store')

        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'deny', 'ui_locale': 'en-US',
        }, form=True, headers=browser)
        self.assertEqual(status, 303)
        callback = parse_qs(urlsplit(headers['Location']).query)
        self.assertEqual(callback['error'], ['access_denied'])
        self.assertEqual(callback['state'], [authorization['state']])
        self.assertEqual(callback['iss'], [self.base])
        self.assertNotIn('code', callback)

        status, _, body = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'allow', 'password': 'do-not-echo-this-password', 'ui_locale': 'ar-SA',
        }, form=True, headers=browser)
        self.assertEqual(status, 401)
        html = body.decode()
        self.assertIn('lang="ar-SA" dir="rtl"', html)
        self.assertIn('role="alert"', html)
        self.assertIn('aria-invalid="true"', html)
        self.assertNotIn('do-not-echo-this-password', html)

        # A completed form can still authorize after a denial and a failed
        # password attempt, and the callback keeps the protocol's state/issuer.
        status, headers, _ = self.request('/oauth/authorize', 'POST', authorization | {
            'decision': 'allow', 'password': password,
        }, form=True, headers=browser)
        self.assertEqual(status, 303)
        self.assertIn('code', parse_qs(urlsplit(headers['Location']).query))

        # An untrusted callback stays local; HTML is only used for browsers.
        invalid = authorization | {'redirect_uri': 'https://attacker.example.com/callback'}
        status, headers, body = self.request('/oauth/authorize?' + urlencode(invalid), headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])
        self.assertNotIn('Location', headers)
        self.assertIn('Unable to connect', body.decode())
        self.assertIn('redirect_uri is not registered', body.decode())
        status, headers, body = self.request('/oauth/authorize?' + urlencode(invalid))
        self.assertEqual(status, 400)
        self.assertEqual(body['error'], 'invalid_request')
        self.assertIn('application/json', headers['Content-Type'])
        status, headers, body = self.request('/oauth/authorize?client_id=a&client_id=b', headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])
        self.assertIn('Unable to connect', body.decode())
        status, headers, body = self.request('/oauth/authorize', 'POST', {}, headers=browser)
        self.assertEqual(status, 400)
        self.assertIn('text/html', headers['Content-Type'])

        # Validated-client errors must still go back through OAuth rather than
        # strand the client on a local error page after the redesign.
        invalid_pkce = authorization | {'code_challenge_method': 'plain'}
        status, headers, _ = self.request('/oauth/authorize?' + urlencode(invalid_pkce), headers=browser)
        self.assertEqual(status, 303)
        callback = parse_qs(urlsplit(headers['Location']).query)
        self.assertEqual(callback['error'], ['invalid_request'])
        self.assertEqual(callback['iss'], [self.base])
        self.assertEqual(callback['state'], [authorization['state']])


if __name__ == '__main__':
    unittest.main()
